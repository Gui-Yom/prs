use plotly::common::{Marker, MarkerSymbol, Mode, Title};
use plotly::layout::{Axis, GridPattern, LayoutGrid, RangeSlider, RowOrder};
use plotly::{Layout, Plot, Scatter};
use stats::StatsLayer;
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
    //let throughput_recorder = Rc::new(RefCell::new(ThroughtputRecorder::default()));
    #[cfg(feature = "trace")]
    let trace_recorder = Rc::new(RefCell::new(stats::TraceRecorder::default()));
    let srtt_recorder = Rc::new(RefCell::new(stats::SrttRecorder::default()));

    let registry = tracing_subscriber::registry();
    //.with(StatsLayer::new(throughput_recorder.clone()))
    #[cfg(feature = "trace")]
    let registry = registry.with(StatsLayer::new(trace_recorder.clone()));

    registry
        .with(StatsLayer::new(srtt_recorder.clone()))
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

    let handler = async move {
        loop {
            let mut client = listener.accept().await;
            info!("New client !");

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

    #[cfg(feature = "trace")]
    {
        /*
            let throughput = Scatter::new(
                throughput_recorder.borrow().timestamps.clone(),
                throughput_recorder.borrow().values.clone(),
            )
            .mode(Mode::Lines)
            .name("Throughtput (kB/s)")
            .line(Line::new().shape(LineShape::Hv));
        */
        let losses = Scatter::new(
            trace_recorder.borrow().lost_timestamps.clone(),
            trace_recorder.borrow().losses.clone(),
        )
        .mode(Mode::Markers)
        .marker(Marker::new().symbol(MarkerSymbol::Cross))
        .name("Losses");

        let acks = Scatter::new(
            trace_recorder.borrow().ack_timestamps.clone(),
            trace_recorder.borrow().acks.clone(),
        )
        .web_gl_mode(true)
        .mode(Mode::Markers)
        .marker(Marker::new().symbol(MarkerSymbol::Cross))
        .name("ACK");

        let srtt = Scatter::new(
            srtt_recorder.borrow().timestamps.clone(),
            srtt_recorder.borrow().values.clone(),
        )
        .mode(Mode::Lines)
        .name("SRTT");

        // Plot
        let mut plot = Plot::new();
        //plot.add_trace(throughput);
        plot.add_trace(losses);
        plot.add_trace(acks);
        plot.add_trace(srtt);

        let layout = Layout::new()
            .grid(
                LayoutGrid::new()
                    .rows(2)
                    .columns(1)
                    .pattern(GridPattern::Independent)
                    .row_order(RowOrder::TopToBottom),
            )
            .x_axis(Axis::new().range_slider(RangeSlider::new().visible(true)))
            .title(Title::new("Transfer trace"));
        plot.set_layout(layout);

        plot.write_html("graph.html");
    }
}
