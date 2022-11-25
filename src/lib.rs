//! UDP control protocol unidirectional server library
//! Fully asynchronous API

use std::collections::{HashMap, VecDeque};
use std::io;
use std::io::Write;
use std::io::{Cursor, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::str::from_utf8_unchecked;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task;
use tokio::time::timeout;
use tracing::{debug, error, trace};

const ACK_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_SIZE: usize = 6;
const MAX_PACKET_SIZE: usize = 1472;
const DATA_SIZE: usize = MAX_PACKET_SIZE - HEADER_SIZE;
/// Window size
const WINDOW_SIZE: usize = 80;
/// Max duplicate ACk to receive before retransmitting, must be < WINDOW_CAP - 1
const MAX_DUP_ACK: i32 = 1;
/// Weight of past srtt estimations
const SRTT_ALPHA: f64 = 0.9;
// TODO find the right value
const SRTT_MAX: Duration = Duration::from_millis(1);

/// Handle to a connected client
#[derive(Debug)]
pub struct UdpcpStream {
    sock: UdpSocket,
    /// Internal buffer used for sending and receiving frames
    // TODO evaluate stack vs heap performance
    frame: [u8; MAX_PACKET_SIZE],
}

impl UdpcpStream {
    /// sock must be correctly configured
    fn new(sock: UdpSocket) -> Self {
        Self {
            sock,
            frame: [0; MAX_PACKET_SIZE],
        }
    }

    /// Format a frame for sending
    async fn send_frame(&mut self, seq: usize, data: &[u8]) -> usize {
        // Write the sequence number
        // Cursor because write! requires an impl of io::Write
        write!(
            Cursor::new(&mut self.frame[..]),
            "{:0>1$}",
            seq,
            HEADER_SIZE
        )
        .unwrap();

        // Write data to the frame
        let data_len = DATA_SIZE.min(data[(seq - 1) * DATA_SIZE..].len());
        self.frame[HEADER_SIZE..data_len + HEADER_SIZE]
            .copy_from_slice(&data[(seq - 1) * DATA_SIZE..(seq - 1) * DATA_SIZE + data_len]);
        self.sock
            .send(&self.frame[..HEADER_SIZE + data_len])
            .await
            .unwrap();
        data_len
    }

    /// Write to the stream, suspends until all data is received by the peer correctly (empty window)
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut seq = 1;
        let mut acked: i32 = 0;
        //let max_seq = buf.len() / DATA_SIZE;
        //let remaining = buf.len() % DATA_SIZE;

        // Send window
        let mut window = 0;

        // Remaining duplicate ack to receive before retransmitting
        let mut dup_ack = MAX_DUP_ACK;
        let mut instants = VecDeque::with_capacity(WINDOW_SIZE);

        let mut srtt = Duration::from_millis(1);

        let mut percent = 0.1;

        while (seq - 1) * DATA_SIZE < buf.len() || window != 0 {
            // Quick update
            if ((seq - 1) * DATA_SIZE) as f32 / buf.len() as f32 > percent {
                debug!("Transfer at {:.3}%", percent * 100.0);
                percent += 0.1;
            }

            // Try sending many frames
            while (seq - 1) * DATA_SIZE < buf.len() && window < WINDOW_SIZE {
                // Create the frame to send
                let data_len = self.send_frame(seq, buf).await;
                instants.push_back(Instant::now());
                trace!(sent = data_len, seq, "Sent frame");

                // Append packet to window
                window += 1;

                // Increment next sequence number
                seq += 1;
            }

            if instants.front().unwrap().elapsed() > srtt {
                trace!(
                    timeout = acked + 1,
                    "Timeout expired, retranmitting oldest packet"
                );
                let _ = self.send_frame((acked + 1) as usize, buf).await;
                *instants.get_mut(0).unwrap() = Instant::now();
            }

            // At this point, either the window is full or we don't have anymore data to send
            // Just wait for an ack to arrive
            'recv: loop {
                match self.sock.try_recv(&mut self.frame) {
                    Ok(n) => {
                        if n == 10 && &self.frame[..3] == b"ACK" {
                            // Read the ack sequence number
                            let ack = unsafe { from_utf8_unchecked(&self.frame[3..9]) }
                                .parse()
                                .unwrap();
                            trace!(rseq = ack, "Received ACK");

                            if ack == acked {
                                // This ack was already received
                                if dup_ack <= 0 {
                                    if window > 0 {
                                        // Exceeded duplicate ack counter
                                        // We only retransmit the packet we know has been lost
                                        trace!(lost = ack + 1, "Retransmitting lost packet");
                                        let _ = self.send_frame((ack + 1) as usize, buf).await;
                                        *instants.get_mut((ack - acked) as usize).unwrap() =
                                            Instant::now();
                                    }
                                    // Reset the counter
                                    dup_ack = MAX_DUP_ACK;
                                } else {
                                    dup_ack -= 1;
                                    trace!("Duplicate ACK");
                                }
                            } else if ack < acked {
                                // This ack is outdated
                                trace!("Ignored ACK");
                            } else {
                                // Valid ack sequence number
                                // Validate everything until this ack
                                //debug!(window, ack, acked, "help pls");
                                let num = (ack as i32 - acked) as usize;
                                window -= num;
                                for _ in 0..num {
                                    let instant = instants.pop_front().unwrap();
                                    srtt = Duration::from_micros(
                                        (SRTT_ALPHA * srtt.as_micros() as f64
                                            + (1.0 - SRTT_ALPHA)
                                                * instant.elapsed().as_micros() as f64)
                                            as u64,
                                    )
                                    .min(SRTT_MAX);
                                }
                                trace!(srtt = srtt.as_micros(), "Recalculating srtt");
                                acked = ack;
                            }
                        } else {
                            error!("Received something other than an ACK");
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        break 'recv;
                    }
                    Err(e) => {
                        error!("Error while receiving : {e}");
                        return Err(e);
                    }
                }
            }
        }
        Ok(buf.len())
    }

    /// Read from the stream
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.recv(buf).await
    }

    /// Stop the connection by sending FIN
    pub async fn stop(&mut self) {
        self.sock.send(b"FIN").await.unwrap();
        debug!("Sent FIN");
    }
}

/// Handle to a server
pub struct UdpcpListener {
    /// Used to get new clients from the acceptor loop
    accept_rx: UnboundedReceiver<UdpcpStream>,
}

impl UdpcpListener {
    pub async fn bind(ip: &str, port: u16) -> io::Result<Self> {
        // If the channel was bounded, we would have a connection backlog limit
        let (tx, rx) = mpsc::unbounded_channel();
        // Spawn the connection acceptor loop
        task::spawn(Self::accept_loop(
            UdpSocket::bind((ip, port)).await?,
            tx,
            port + 1,
        ));

        Ok(Self { accept_rx: rx })
    }

    async fn accept_loop(sock: UdpSocket, tx: UnboundedSender<UdpcpStream>, mut next_port: u16) {
        // Clients we are having a handshake with
        let mut clients = HashMap::<SocketAddr, (Instant, UdpSocket)>::new();
        // Reception buffer
        let mut buf = [0; 4];
        // Min wait duration for the next event
        let mut wait = Duration::from_secs(u64::MAX);

        'main: loop {
            debug!(wait = wait.as_millis(), "Waiting");
            let client_addr = match timeout(wait, sock.recv_from(&mut buf)).await {
                Ok(Ok((_, client_addr))) => client_addr,
                Ok(Err(e)) => {
                    error!("Got error: {e}");
                    return;
                }
                Err(_) => {
                    for (addr, (instant, client)) in clients.iter_mut() {
                        if instant.elapsed() >= ACK_TIMEOUT {
                            debug!("ACK timeout for client {addr}, sending SYNACK again");
                            let synack =
                                format!("SYN-ACK{}\0", client.local_addr().unwrap().port());
                            sock.send_to(synack.as_bytes(), addr).await.unwrap();
                            // Update synack send time
                            *instant = Instant::now();
                        }
                    }
                    continue;
                }
            };

            // Handle the received message
            match &buf[..] {
                [b'S', b'Y', b'N', 0] => {
                    if let Some((instant, client)) = clients.get_mut(&client_addr) {
                        debug!("Received SYN again, client {client_addr} did not receive our SYNACK, sending it again");
                        let synack = format!("SYN-ACK{}\0", client.local_addr().unwrap().port());
                        sock.send_to(synack.as_bytes(), client_addr).await.unwrap();

                        // Update connection state
                        *instant = Instant::now();
                    } else {
                        let synack = format!("SYN-ACK{next_port}\0");
                        sock.send_to(synack.as_bytes(), client_addr).await.unwrap();
                        debug!("Sent {synack} to {client_addr}");

                        // Create the local udp socket at the moment we send SYN-ACK
                        let client = UdpSocket::bind((IpAddr::from([0, 0, 0, 0]), next_port))
                            .await
                            .unwrap();
                        client.connect(client_addr).await.unwrap();

                        // Update connection state
                        let now = Instant::now();
                        clients.insert(client_addr, (now, client));

                        next_port += 1;
                    }
                }
                [b'A', b'C', b'K', 0] => {
                    if let Some((_, client)) = clients.remove(&client_addr) {
                        debug!("Received ACK from {client_addr}, handshake finished");
                        tx.send(UdpcpStream::new(client)).unwrap();
                    } else {
                        error!("Unexpected ACK");
                        return;
                    }
                }
                _ => {
                    error!("Unexpected data : {buf:?}");
                    return;
                }
            }

            if !clients.is_empty() {
                // Find the farthest instant in time
                let mut oldest = Duration::from_secs(0);
                for (_, (instant, _)) in clients.iter() {
                    oldest = oldest.max(instant.elapsed());
                }
                wait = ACK_TIMEOUT - oldest.min(ACK_TIMEOUT);
            } else {
                wait = Duration::from_secs(u64::MAX);
            }
        }
    }

    pub async fn accept(&mut self) -> UdpcpStream {
        self.accept_rx
            .recv()
            .await
            .expect("Channel has been closed somehow")
    }
}
