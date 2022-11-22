//! UDP control protocol unidirectional server library
//! Fully asynchronous API

use std::collections::{HashMap, VecDeque};
use std::io;
use std::io::Cursor;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::str::from_utf8_unchecked;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task;
use tokio::time::timeout;
use tracing::{debug, error};

const ACK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PACKET_SIZE: usize = 1024;
const WINDOW_CAP: usize = 4;
const MAX_DUP_ACK: i32 = 2;

/// Handle to a connected client
#[derive(Debug)]
pub struct UdpcpStream {
    sock: UdpSocket,
    /// Internal buffer used for sending and receiving frames
    frame: [u8; MAX_PACKET_SIZE],
    /// Current sequence number
    seq: u32,
    /// Last acked sequence number
    ack_seq: u32,
    /// Send window
    window: VecDeque<(u32, Box<[u8]>)>,
}

impl UdpcpStream {
    /// sock must be correctly configured
    fn new(sock: UdpSocket) -> Self {
        Self {
            sock,
            frame: [0; 1024],
            seq: 1,
            ack_seq: 1,
            window: VecDeque::with_capacity(WINDOW_CAP),
        }
    }

    /// Write to the stream, blocks for the data to arrive in the window
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut ptr = buf;
        let mut retransmit_threshold = MAX_DUP_ACK;
        'main: while !ptr.is_empty() || !self.window.is_empty() {
            // Try sending many frames
            while self.window.len() < self.window.capacity() && !ptr.is_empty() {
                // Write the sequence number
                // Cursor because write! requires an impl of io::Write
                write!(Cursor::new(&mut self.frame[..]), "{:0>6}", self.seq).unwrap();
                let mut n = 6;

                // Write data to the frame
                n += (MAX_PACKET_SIZE - 6).min(ptr.len());
                self.frame[6..n].copy_from_slice(&ptr[..n - 6]);
                ptr = &ptr[n - 6..];

                // Append packet to window
                self.window
                    .push_back((self.seq, self.frame[..n].to_vec().into_boxed_slice()));

                // Send packet
                self.sock.send(&self.frame[..n]).await.unwrap();
                debug!(n, seq = self.seq, "Sent bytes");

                // Increment next sequence number
                self.seq += 1;
            }
            match self.sock.recv(&mut self.frame).await {
                Ok(n) => {
                    if n == 10 && &self.frame[..3] == b"ACK" {
                        let ackseq: u32 = unsafe { from_utf8_unchecked(&self.frame[3..9]) }
                            .parse()
                            .unwrap();

                        debug!(seq = ackseq, "Received ACK");

                        if ackseq == self.ack_seq {
                            if retransmit_threshold <= 0 {
                                debug!(n = self.window.len(), "Retransmitting window");
                                for (_, packet) in self.window.iter() {
                                    self.sock.send(packet).await.unwrap();
                                }
                                retransmit_threshold = MAX_DUP_ACK;
                            } else {
                                retransmit_threshold -= 1;
                                debug!(dup = retransmit_threshold, "Duplicate ACK");
                            }
                        } else if ackseq < self.ack_seq {
                            debug!("Ignored ACK");
                        } else {
                            while let Some((seq, packet)) = self.window.pop_front() {
                                if ackseq >= seq {
                                    debug!(seq, "Validated frame");
                                    self.ack_seq = seq;
                                } else {
                                    self.window.push_front((seq, packet));
                                    break;
                                }
                            }
                        }
                    } else {
                        error!("Received something other than an ACK");
                    }
                }
                Err(e) => {
                    error!("Error while receiving : {e}");
                    return Err(e);
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
                        dbg!(next_port);
                        let synack = format!("SYN-ACK{next_port}\0");
                        sock.send_to(synack.as_bytes(), client_addr).await.unwrap();
                        debug!("Sent SYNACK to {client_addr}");

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
