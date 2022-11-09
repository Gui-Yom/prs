use tp3rs::TcpUdpStream;
use tracing::{debug, info, Level};
use tracing_subscriber::fmt;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    fmt().with_max_level(Level::DEBUG).init();

    let server_ip = "127.0.0.1";
    let port = 5001;
    info!("Server addr is : {server_ip}:{port}");

    let mut client = TcpUdpStream::connect(server_ip, port)
        .await
        .expect("Connection failed");
    info!("Connected !");

    let mut buf = [0; 1024];
    loop {
        let n = client.read(&mut buf).await.unwrap();
        debug!("Read {n} bytes");
    }
}
