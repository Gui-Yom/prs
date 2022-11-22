use plotly::common::{Line, LineShape, Mode, Title};
use plotly::layout::{Axis, RangeSlider};
use plotly::{Layout, Plot, Scatter};
use stats::{StatsLayer, ThroughtputRecorder};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::{fs, select, task};
use tp3rs::UdpcpListener;
use tracing::{debug, info};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

pub mod stats;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let recorder = Rc::new(RefCell::new(ThroughtputRecorder::default()));
    tracing_subscriber::registry()
        .with(StatsLayer::new(recorder.clone()))
        .with(
            fmt::layer().compact().with_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::DEBUG.into())
                    .from_env_lossy(),
            ),
        )
        .try_init()
        .unwrap();

    let port = 5001;
    let mut listener = UdpcpListener::bind("0.0.0.0", port).await.unwrap();
    info!("Listening on 0.0.0.0:{port}");

    //let buf = Arc::new(fs::read("Cargo.lock").await.unwrap());
    //debug!("File is {} bytes", buf.len());

    let handler = async move {
        loop {
            let mut client = listener.accept().await;
            info!("New client !");
            //let bufr = buf.clone();
            task::spawn(async move {
                let buf = {
                    let mut tmp = [0; 256];
                    let n = client.read(&mut tmp).await.unwrap();
                    let fname = String::from_utf8_lossy(&tmp[..n - 1]).to_string();
                    debug!(fname, "Received file name");
                    fs::read(&fname).await.unwrap()
                };

                // TODO read file once (hashmap)

                client.write(&buf).await.unwrap();
                info!("Copy finished");
                client.stop().await;
            });
        }
    };

    select! {
        _ = handler => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    let trace = Scatter::new(
        recorder.borrow().timestamps.clone(),
        recorder.borrow().values.clone(),
    )
    .mode(Mode::Lines)
    .name("throughtput")
    .line(Line::new().shape(LineShape::Hv));

    let mut plot = Plot::new();
    plot.add_trace(trace);

    let layout = Layout::new()
        .x_axis(Axis::new().range_slider(RangeSlider::new().visible(true)))
        .title(Title::new("Throughput over time"));
    plot.set_layout(layout);

    plot.write_html("graph.html");
}
