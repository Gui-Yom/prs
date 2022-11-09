use std::sync::Arc;
use tokio::{fs, task};
use tp3rs::TcpUdpListener;
use tracing::{debug, info, Level};
use tracing_subscriber::fmt;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    fmt().with_max_level(Level::DEBUG).init();

    let port = 5001;
    let mut listener = TcpUdpListener::bind("0.0.0.0", port).await.unwrap();
    info!("Listening on 0.0.0.0:{port}");

    let buf = Arc::new(fs::read("Cargo.lock").await.unwrap());
    debug!("File is {} bytes", buf.len());

    loop {
        let mut client = listener.accept().await;
        let bufr = buf.clone();
        task::spawn(async move {
            client.write(bufr.as_ref()).await.unwrap();
            client.flush().await;
            info!("Copy finished");
        });
    }
}
