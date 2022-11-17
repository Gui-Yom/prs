use plotly::common::{Line, LineShape, Mode, Title};
use plotly::{Layout, Plot, Scatter};
use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tokio::{fs, select, task};
use tp3rs::stats::{StatsLayer, WindowStatsRecorder};
use tp3rs::TcpUdpListener;
use tracing::field::{Field, Visit};
use tracing::{debug, info, info_span, Event, Instrument, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, Layer, Registry};

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let recorder = Rc::new(RefCell::new(WindowStatsRecorder::default()));
    tracing_subscriber::registry()
        .with(fmt::layer().compact())
        .with(StatsLayer::new(recorder.clone()))
        .try_init()
        .unwrap();

    let port = 5001;
    let mut listener = TcpUdpListener::bind("0.0.0.0", port).await.unwrap();
    info!("Listening on 0.0.0.0:{port}");

    //let buf = Arc::new(fs::read("Cargo.lock").await.unwrap());
    //debug!("File is {} bytes", buf.len());

    let handler = async move {
        loop {
            let client = listener.accept().await;
            //let bufr = buf.clone();
            task::spawn(
                async move {
                    let buf = {
                        let mut tmp = [0; 1024];
                        let n = client.read(&mut tmp).await.unwrap();
                        let fname = String::from_utf8_lossy(&tmp[..n - 1]).to_string();
                        debug!(fname, "Received file name");
                        fs::read(&fname).await.unwrap()
                    };

                    // TODO read file once (hashmap)

                    client.start();

                    client.write(&buf).await.unwrap();
                    client.flush().await;
                    info!("Copy finished");

                    client.stop();
                }
                .instrument(info_span!("client task")),
            );
        }
    };

    select! {
        _ = handler => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    let trace = Scatter::new(
        (0..recorder.borrow().values.len()).collect(),
        recorder.borrow().values.clone(),
    )
    .mode(Mode::Lines)
    .name("window size")
    .line(Line::new().shape(LineShape::Hv));

    let mut plot = Plot::new();
    plot.add_trace(trace);

    let layout = Layout::new().title(Title::new("Window size over time"));
    plot.set_layout(layout);

    plot.write_html("graph.html");
}
