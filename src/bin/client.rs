use std::net::{IpAddr, SocketAddr};
use tracing::{info, Level};
use tracing_subscriber::fmt;
use tp3rs::TcpUdpStream;

#[async_std::main]
async fn main() {
    fmt().with_max_level(Level::DEBUG).init();

    let server_ip = IpAddr::from([127, 0, 0, 1]);
    let server_addr = SocketAddr::new(server_ip, 5001);
    info!("Server addr is : {server_addr}");

    let client = TcpUdpStream::connect(server_addr).await.expect("Connection failed");
    info!("Connected !");
}
