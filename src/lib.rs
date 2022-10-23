//! TCP over UDP library
//! Fully async

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::io::{Error, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::ops::Sub;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use async_std::channel::{Receiver, RecvError, Sender};
use async_std::io::{timeout, Write, WriteExt};
use async_std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use async_std::task::JoinHandle;
use tracing::{debug, error};

const ACK_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PACKET_SIZE: usize = 1024;
const MAX_RETRIES: usize = 3;

pub struct TcpUdpStream {
    sock: UdpSocket,
}

impl TcpUdpStream {
    pub async fn connect<A: ToSocketAddrs + Clone>(addr: A) -> io::Result<Self> {
        let sock = UdpSocket::bind((IpAddr::from([127, 0, 0, 1]), 0)).await?;
        sock.connect(addr.clone()).await?;
        // Send SYN in a loop until we get SYNACK
        let mut tries = 0;
        let port = loop {
            sock.send(b"SYN").await?;
            tries += 1;
            debug!("Sent SYN");
            let mut buf = vec![0; 11];

            // Wait for at most ACK_TIMEOUT
            match timeout(ACK_TIMEOUT, sock.recv(&mut buf)).await {
                Ok(n) => {
                    let msg = String::from_utf8_lossy(&buf[..n]);
                    if !msg.starts_with("SYNACK") {
                        error!("Received gibberish : {buf:?}");
                        return Err(Error::from(ErrorKind::InvalidData));
                    }
                    // Extract port number
                    let port = msg[7..n].parse().expect("Invalid port number");
                    debug!("Received SYNACK, data port is {port}");
                    sock.send(b"ACK").await?;
                    debug!("Sent ACK");
                    break port;
                }
                Err(e) => {
                    if e.kind() == ErrorKind::TimedOut && tries < MAX_RETRIES {
                        debug!("SYNACK timeout, sending SYN again");
                        continue;
                    } else {
                        error!("Got error : {e}");
                        return Err(e);
                    }
                }
            }
        };

        // Connect the socket to the new port
        let mut a: SocketAddr = addr.to_socket_addrs().await?.next().unwrap();
        a.set_port(port);
        sock.connect(a).await?;

        Ok(Self {
            sock
        })
    }

    pub async fn write(&self, buf: &[u8]) -> io::Result<()> {
        let mut ack = vec![0; 3];
        for chunk in buf.chunks(MAX_PACKET_SIZE) {
            let mut tries = 0;
            loop {
                self.sock.send(chunk).await?;
                tries += 1;
                // Receive ACK
                match timeout(ACK_TIMEOUT, self.sock.recv(&mut ack)).await {
                    Ok(_) => {
                        if buf == [b'A', b'C', b'K'] {
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() == ErrorKind::TimedOut && tries < MAX_RETRIES {
                            continue;
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /*
    async fn read(&self, buf: &mut [u8]) {
        self.sock.recv()
    }*/
}

pub struct TcpUdpListener {
    accept_rx: Receiver<TcpUdpStream>,
    accept_task: JoinHandle<()>,
}

impl TcpUdpListener {
    pub async fn bind<A: ToSocketAddrs + Clone>(addr: A) -> io::Result<Self> {
        let port = addr.clone().to_socket_addrs().await?.next().unwrap().port();

        // If the channel was bounded, we would have a connection backlog limit
        let (tx, rx) = async_std::channel::unbounded();
        let accept_task = async_std::task::spawn(Self::accept_loop(UdpSocket::bind(addr).await?, tx, port));

        Ok(Self {
            accept_rx: rx,
            accept_task,
        })
    }

    async fn accept_loop(sock: UdpSocket, tx: Sender<TcpUdpStream>, start_port: u16) {
        // Clients we are having a handshake with
        let mut clients = HashMap::<SocketAddr, (u16, Instant)>::new();
        // The next port to be attributed
        let mut next_port = start_port;
        // Reception buffer
        let mut buf = vec![0; 3];
        // Min wait duration for the next event
        let mut wait = Duration::from_secs(u64::MAX);

        'main: loop {
            debug!("Waiting for at most {} ms", wait.as_millis());
            let client_addr = match timeout(ACK_TIMEOUT.min(wait), sock.recv_from(&mut buf)).await {
                Ok((_, client_addr)) => {
                    client_addr
                }
                Err(e) if e.kind() == ErrorKind::TimedOut => {
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
                Err(e) => {
                    error!("Error trying to receive data ({}) : {e}", e.kind());
                    return;
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
                    if let Some((port, instant)) = clients.get_mut(&client_addr) {
                        debug!("Received ACK from {client_addr}, handshake finished");
                        clients.remove(&client_addr);

                        // Create the client udp socket
                        let client = UdpSocket::bind((IpAddr::from([127, 0, 0, 1]), port)).await.unwrap();
                        client.connect(client_addr);
                        tx.send(TcpUdpStream { sock: client }).await.unwrap();
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

            if clients.len() > 0 {
                // Find the farthest instant in time
                let mut oldest = Duration::from_secs(0);
                for (_, (_, instant)) in clients.iter() {
                    oldest = oldest.max(instant.elapsed());
                }
                wait = (ACK_TIMEOUT - oldest.min(ACK_TIMEOUT));
            } else {
                wait = Duration::from_secs(u64::MAX);
            }
        }
    }

    pub async fn accept(&self) -> Result<TcpUdpStream, RecvError> {
        self.accept_rx.recv().await
    }
}
