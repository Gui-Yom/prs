use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use async_std::io::{ReadExt, timeout};
use async_std::net::UdpSocket;
use tracing::{debug, error, info, Level};
use tracing_subscriber::{fmt, FmtSubscriber};
use tp3rs::TcpUdpListener;

const ACK_TIMEOUT: Duration = Duration::from_secs(5);

#[async_std::main]
async fn main() {
    fmt().with_max_level(Level::DEBUG).init();

    let server_addr = SocketAddr::new(Ipv4Addr::new(0, 0, 0, 0).into(), 5001);
    let listener = TcpUdpListener::bind(server_addr).await.unwrap();
    info!("Listening on {server_addr}");

    loop {
        let client = listener.accept().await.unwrap();
        async_std::task::spawn(async move {
            let mut file = async_std::fs::File::open("Cargo.lock").await?;
            let mut buf = vec![0; 1024];
            loop {
                let n = file.read(&mut buf).await?;
                if n > 0 {
                    client.write(&buf[..n]).await?;
                } else {
                    break;
                }
            }
        });
    }
}