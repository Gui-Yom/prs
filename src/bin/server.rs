use plotly::common::{Line, LineShape, Mode, Title};
use plotly::{Layout, Plot, Scatter};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tokio::{fs, select, task};
use tp3rs::TcpUdpListener;
use tracing::field::{Field, Visit};
use tracing::{debug, info, info_span, Event, Instrument, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, Layer};

#[derive(Default)]
struct WindowStatsRecorder {
    next: String,
    values: Vec<i64>,
}

impl Visit for WindowStatsRecorder {
    fn record_i64(&mut self, field: &Field, value: i64) {
        println!("field: {} = {value}", field.name());
        match field.name() {
            "window_len" => {
                self.values.push(value);
            }
            _ => {}
        }
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.record_i64(field, value as i64);
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.record_i64(field, value as i64);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_i64(field, value as i64);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.next == value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {}
}

trait StatsRecorder: Visit + Default {}

impl<T: Visit + Default> StatsRecorder for T {}

struct StatsLayer<R> {
    data: Arc<Mutex<R>>,
}

impl<R: StatsRecorder> Default for StatsLayer<R> {
    fn default() -> Self {
        Self {
            data: Arc::new(Mutex::new(R::default())),
        }
    }
}

impl<R: StatsRecorder + 'static, S: Subscriber> Layer<S> for StatsLayer<R> {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        event.record(self.data.lock().as_deref_mut().unwrap());
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let stats = StatsLayer::<WindowStatsRecorder>::default();
    let recorder = stats.data.clone();
    tracing_subscriber::registry()
        .with(fmt::layer().pretty())
        .with(stats)
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

    let locked = recorder.lock().unwrap();

    let trace = Scatter::new((0..=locked.values.len()).collect(), locked.values.clone())
        .mode(Mode::Lines)
        .name("window size")
        .line(Line::new().shape(LineShape::Hv));

    let mut plot = Plot::new();
    plot.add_trace(trace);

    let layout = Layout::new().title(Title::new("Window size over time"));
    plot.set_layout(layout);

    plot.write_html("graph.html");
}
