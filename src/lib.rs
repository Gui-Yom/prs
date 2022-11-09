//! TCP over UDP library
//! Fully async

use std::collections::{HashMap, VecDeque};
use std::io;
use std::io::{Error, ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::timeout;
use tokio::{select, task};
use tracing::{debug, error};

const ACK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PACKET_SIZE: usize = 1024;
const MAX_RETRIES: usize = 3;
const WINDOW_CAP: usize = 4;

#[derive(Debug)]
struct Inner {
    sndbuf: Mutex<VecDeque<u8>>,
    rcvbuf: Mutex<VecDeque<u8>>,
    has_snd_data: Notify,
    has_rcv_data: Notify,
    window: Mutex<VecDeque<(u32, Vec<u8>)>>,
}

#[derive(Debug)]
pub struct TcpUdpStream(Arc<Inner>);

impl TcpUdpStream {
    /// sock must be correctly configured
    fn new(sock: UdpSocket) -> Self {
        let inner2 = Arc::new(Inner {
            sndbuf: Mutex::new(VecDeque::with_capacity(8 * MAX_PACKET_SIZE)),
            rcvbuf: Mutex::new(VecDeque::with_capacity(8 * MAX_PACKET_SIZE)),
            has_snd_data: Notify::new(),
            has_rcv_data: Notify::new(),
            window: Mutex::new(VecDeque::with_capacity(WINDOW_CAP)),
        });

        let inner = inner2.clone();
        task::spawn(async move {
            let Inner {
                sndbuf,
                rcvbuf,
                has_snd_data,
                has_rcv_data,
                window,
            } = &*inner;

            let mut frame = [0; MAX_PACKET_SIZE];
            let mut snd_seq = 100000;
            let mut rcv_seq = 100000;

            let mut rejected = false;
            loop {
                let window_len = window.lock().await.len();
                debug!(window_len, "New loop");
                select! {
                    res = sock.recv(&mut frame) => {
                        match res {
                            Ok(n) => {
                                if n == 9 && &frame[..3] == b"ACK" {
                                    let ackseq: u32 = String::from_utf8_lossy(&frame[3..9]).parse().unwrap();
                                    let mut window = window.lock().await;
                                    if let Some((seq, packet)) = window.pop_front() {
                                        if ackseq == seq {
                                            debug!(ackseq, "Got ACK");
                                        } else {
                                            debug!(ackseq, "Wrong ACK");
                                            // Append packet we just popped
                                            window.push_front((seq, packet));
                                            debug!(n = window.len(), "Retransmitting window");
                                            for (seq, packet) in window.iter() {
                                                sock.send(packet).await.unwrap();
                                            }
                                        }
                                    }
                                } else {
                                    let frameseq: u32 = String::from_utf8_lossy(&frame[..6]).parse().unwrap();
                                    if frameseq != rcv_seq {
                                        debug!(seq = frameseq, "Received unexpected frame");
                                        // Send ack when reading a packet
                                        sock.send(format!("ACK{}", rcv_seq - 1).as_bytes()).await.unwrap();
                                        debug!(seq = rcv_seq - 1, "Sent ACK");
                                    } else {
                                        if frameseq == 100008 && !rejected {
                                            rejected = true;
                                        } else {
                                            let mut rcv = rcvbuf.lock().await;
                                            rcv.write(&frame[6..n]).unwrap();
                                            debug!(n = n-6, "Received data");
                                            // Send ack when reading a packet
                                            sock.send(format!("ACK{frameseq}").as_bytes()).await.unwrap();
                                            debug!(seq = frameseq, "Sent ACK");
                                            has_rcv_data.notify_one();
                                            rcv_seq += 1;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Got error : {e}");
                                return;
                            }
                        }
                    }
                    _ = has_snd_data.notified() => {
                        let mut window = window.lock().await;
                        if window.len() < WINDOW_CAP {
                            debug!("Notified (send) !");
                            let mut buf = sndbuf.lock().await;
                            if buf.len() > 0 {
                                let fmt = format!("{snd_seq}");
                                let mut n = fmt.len();
                                frame[..n].copy_from_slice(fmt.as_bytes());
                                n += buf.read(&mut frame[n..]).unwrap();
                                if n < frame.len() {
                                    n += buf.read(&mut frame[n..]).unwrap();
                                }
                                // Append packet to window
                                window.push_back((snd_seq, frame[..n].to_vec()));
                                // Send packet
                                sock.send(&frame[..n]).await.unwrap();
                                debug!(n, seq = snd_seq, "Sent bytes");
                                snd_seq += 1;
                            } else {
                                debug!("We got notified but no data in sndbuf");
                            }
                        } else {
                            debug!("Window is full, not sending");
                        }
                    }
                }

                let window = window.lock().await;
                if window.len() < WINDOW_CAP && sndbuf.lock().await.len() > 0 {
                    has_snd_data.notify_one();
                }
            }
        });

        TcpUdpStream(inner2)
    }

    pub async fn connect(ip: &str, port: u16) -> io::Result<Self> {
        let sock = UdpSocket::bind((IpAddr::from([127, 0, 0, 1]), 0)).await?;
        sock.connect((ip, port)).await?;
        // Send SYN in a loop until we get SYNACK
        let mut tries = 0;
        let port = loop {
            sock.send(b"SYN").await?;
            tries += 1;
            debug!("Sent SYN");
            let mut buf = vec![0; 11];

            // Wait for at most ACK_TIMEOUT
            match timeout(ACK_TIMEOUT, sock.recv(&mut buf)).await {
                Ok(Ok(n)) => {
                    let msg = String::from_utf8_lossy(&buf[..n]);
                    if !msg.starts_with("SYNACK") {
                        error!("Received gibberish : {buf:?}");
                        return Err(Error::from(ErrorKind::InvalidData));
                    }
                    // Extract port number
                    dbg!(msg.clone());
                    let port = msg[6..n].parse().expect("Invalid port number");
                    debug!("Received SYNACK, data port is {port}");
                    sock.send(b"ACK").await?;
                    debug!("Sent ACK");
                    break port;
                }
                Ok(Err(e)) => {
                    error!("Got error: {e}");
                    return Err(e);
                }
                Err(_) => {
                    if tries < MAX_RETRIES {
                        debug!("SYNACK timeout, sending SYN again");
                        continue;
                    } else {
                        error!("Retransmission limit reached");
                        return Err(Error::from(ErrorKind::TimedOut));
                    }
                }
            }
        };

        // Connect the socket to the new port
        sock.connect((ip, port)).await?;

        Ok(Self::new(sock))
    }

    /// Write to the stream
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut snd = self.0.sndbuf.lock().await;
        let n = snd.write(buf)?;
        self.0.has_snd_data.notify_one();
        Ok(n)
    }

    pub async fn flush(&mut self) {
        while !self.0.sndbuf.lock().await.is_empty() && !self.0.window.lock().await.is_empty() {
            task::yield_now().await;
        }
    }

    /// Read from the stream
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        if self.0.rcvbuf.lock().await.len() == 0 {
            self.0.has_rcv_data.notified().await;
            debug!("Notified (receive) !");
        }
        let mut rcv = self.0.rcvbuf.lock().await;
        let mut n = rcv.read(buf)?;
        if n < buf.len() {
            n += rcv.read(&mut buf[n..])?;
        }
        Ok(n)
    }
}

pub struct TcpUdpListener {
    /// Used to get new clients from the acceptor loop
    accept_rx: UnboundedReceiver<TcpUdpStream>,
}

impl TcpUdpListener {
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

    async fn accept_loop(sock: UdpSocket, tx: UnboundedSender<TcpUdpStream>, mut next_port: u16) {
        // Clients we are having a handshake with
        let mut clients = HashMap::<SocketAddr, (u16, Instant)>::new();
        // Reception buffer
        let mut buf = [0; 3];
        // Min wait duration for the next event
        let mut wait = Duration::from_secs(u64::MAX);

        'main: loop {
            debug!("Waiting for at most {} ms", wait.as_millis());
            let client_addr = match timeout(wait, sock.recv_from(&mut buf)).await {
                Ok(Ok((_, client_addr))) => client_addr,
                Ok(Err(e)) => {
                    error!("Got error: {e}");
                    return;
                }
                Err(_) => {
                    for (addr, (port, instant)) in clients.iter_mut() {
                        if instant.elapsed() >= ACK_TIMEOUT {
                            debug!("ACK timeout for client {addr}, sending SYNACK again");
                            let synack = format!("SYNACK{port}");
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
                [b'S', b'Y', b'N'] => {
                    if let Some((port, instant)) = clients.get_mut(&client_addr) {
                        debug!("Received SYN again, client {client_addr} did not receive our SYNACK, sending it again");
                        let synack = format!("SYNACK{port}");
                        sock.send_to(synack.as_bytes(), client_addr).await.unwrap();

                        // Update connection state
                        *instant = Instant::now();
                    } else {
                        dbg!(next_port);
                        let synack = format!("SYNACK{next_port}");
                        sock.send_to(synack.as_bytes(), client_addr).await.unwrap();
                        debug!("Sent SYNACK to {client_addr}");

                        // Update connection state
                        let now = Instant::now();
                        clients.insert(client_addr, (next_port, now));

                        next_port += 1;
                    }
                }
                [b'A', b'C', b'K'] => {
                    let mut removed = None;
                    if let Some((port, instant)) = clients.get_mut(&client_addr) {
                        debug!("Received ACK from {client_addr}, handshake finished");
                        removed = Some(&client_addr);

                        // Create the client udp socket

                        let client = UdpSocket::bind((IpAddr::from([127, 0, 0, 1]), *port))
                            .await
                            .unwrap();
                        client.connect(client_addr).await.unwrap();
                        tx.send(TcpUdpStream::new(client)).unwrap();
                    } else {
                        error!("Unexpected ACK");
                        return;
                    }
                    if let Some(removed) = removed {
                        clients.remove(removed);
                    }
                }
                _ => {
                    error!("Unexpected data : {buf:?}");
                    return;
                }
            }

            if clients.len() > 0 {
                // Find the farthest instant in time
                let mut oldest = Duration::from_secs(0);
                for (_, (_, instant)) in clients.iter() {
                    oldest = oldest.max(instant.elapsed());
                }
                wait = ACK_TIMEOUT - oldest.min(ACK_TIMEOUT);
            } else {
                wait = Duration::from_secs(u64::MAX);
            }
        }
    }

    pub async fn accept(&mut self) -> TcpUdpStream {
        self.accept_rx
            .recv()
            .await
            .expect("Channel has been closed somehow")
    }
}
