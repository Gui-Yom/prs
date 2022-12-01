use plotly::common::{Line, LineShape};
use tokio::{fs, select, task};
use tp3rs::UdpcpListener;
use tracing::{debug, info};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

#[cfg(feature = "trace")]
pub mod metrics;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let registry = tracing_subscriber::registry();

    // Register matrics
    #[cfg(feature = "trace")]
    let (srtt_metric, trace_recorder, throughput_metric, registry) = {
        let srtt_layer = metrics::MetricLayer::<metrics::SrttMetric>::new();
        let trace_layer = metrics::MetricLayer::<metrics::TraceRecorder>::new();
        let throughput_layer = metrics::MetricLayer::<metrics::ThroughputMetric>::new();
        (
            srtt_layer.metric(),
            trace_layer.metric(),
            throughput_layer.metric(),
            registry
                .with(srtt_layer)
                .with(trace_layer)
                .with(throughput_layer),
        )
    };

    registry
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
        // Acceptor loop
        loop {
            let mut client = listener.accept().await;
            info!("New client !");

            // Client's task
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

    // Cancel the acceptor loop on ctrl-c
    select! {
        _ = handler => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    // Generate graphs before exiting

    #[cfg(feature = "trace")]
    {
        use plotly::common::{AxisSide, Marker, MarkerSymbol, Mode, Title};
        use plotly::configuration::{ImageButtonFormats, ToImageButtonOptions};
        use plotly::layout::{Axis, RangeSlider};
        use plotly::Configuration;
        use plotly::{Layout, Plot, Scatter};

        // https://plotly.com/javascript/plotly-fundamentals/
        // https://igiagkiozis.github.io/plotly/content/recipes/subplots/multiple_axes.html

        let (sent, ack, dup, to) = {
            let trace = trace_recorder.lock().unwrap();
            (
                Scatter::new(trace.timestamps_sent.clone(), trace.sent.clone())
                    .mode(Mode::Markers)
                    .web_gl_mode(true)
                    .marker(Marker::new().symbol(MarkerSymbol::Cross))
                    .name("Sent"),
                Scatter::new(trace.timestamps_ack.clone(), trace.acks.clone())
                    .mode(Mode::Markers)
                    .web_gl_mode(true)
                    .marker(Marker::new().symbol(MarkerSymbol::AsteriskOpen))
                    .name("ACK received"),
                Scatter::new(trace.timestamps_dup.clone(), trace.dup.clone())
                    .mode(Mode::Markers)
                    .web_gl_mode(true)
                    .name("Retransmit (dup acks)"),
                Scatter::new(trace.timestamps_to.clone(), trace.to.clone())
                    .mode(Mode::Markers)
                    .web_gl_mode(true)
                    .name("Retransmit (timeouts)"),
            )
        };

        let srtt = {
            let srtt = srtt_metric.lock().unwrap();
            Scatter::new(srtt.timestamps.clone(), srtt.values.clone())
                .mode(Mode::Lines)
                .web_gl_mode(true)
                .name("SRTT")
                .y_axis("y2")
        };

        let throughput = {
            let mut throughput = throughput_metric.lock().unwrap();
            throughput.timestamps.pop();
            throughput.values.pop();
            Scatter::new(throughput.timestamps.clone(), throughput.values.clone())
                .mode(Mode::Lines)
                .web_gl_mode(true)
                .line(Line::new().shape(LineShape::Hv))
                .name("Throughput")
                .y_axis("y3")
        };

        // Plot
        let mut plot = Plot::new();
        plot.add_trace(srtt);
        plot.add_traces(vec![sent, ack, dup, to]);
        plot.add_trace(throughput);

        plot.set_layout(
            Layout::new()
                .y_axis(Axis::new().title(Title::new("sequence number")))
                .y_axis2(
                    Axis::new()
                        .title(Title::new("SRTT (µs)"))
                        .overlaying("y")
                        .side(AxisSide::Right)
                        .domain(&[0., 5000.]),
                )
                .y_axis3(
                    Axis::new()
                        .title(Title::new("Throughput (Mo/s)"))
                        .overlaying("y")
                        .side(AxisSide::Left),
                )
                //.x_axis(Axis::new().range_slider(RangeSlider::new().visible(true)))
                .title(Title::new("Transfer trace")),
        );

        plot.set_configuration(
            Configuration::new()
                .fill_frame(true)
                .to_image_button_options(
                    ToImageButtonOptions::new().format(ImageButtonFormats::Svg),
                )
                .scroll_zoom(true),
        );

        plot.write_html("graph.html");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_this() {}
}
