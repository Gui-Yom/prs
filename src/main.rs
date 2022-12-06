use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio::{fs, select, task};
use tracing::{debug, error, info};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};
use udcp::UdcpListener;

#[cfg(feature = "trace")]
use udcp::metrics;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let registry = tracing_subscriber::registry();

    // Register matrics
    #[cfg(feature = "trace")]
    let (trace_recorder, throughput_metric, registry) = {
        //let srtt_layer = metrics::MetricLayer::<metrics::SrttMetric>::new();
        let trace_layer = metrics::MetricLayer::<metrics::TraceRecorder>::new();
        let throughput_layer = metrics::MetricLayer::<metrics::ThroughputMetric>::new();
        (
            //srtt_layer.metric(),
            trace_layer.metric(),
            throughput_layer.metric(),
            registry
                //.with(srtt_layer)
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
    let mut listener = UdcpListener::bind("0.0.0.0", port).await.unwrap();
    info!("Listening on 0.0.0.0:{port}");

    #[cfg(feature = "trace")]
    let (trace_server, trace_data) = {
        let trace_server = TcpListener::bind(("0.0.0.0", port - 1)).await.unwrap();
        debug!("Trace server is listening on 0.0.0.0:{}", port - 1);
        (
            Arc::new(trace_server),
            Arc::new(Mutex::new(metrics::Data::new())),
        )
    };

    #[cfg(feature = "trace")]
    let trace_data_c = trace_data.clone();

    let handler = async move {
        // Acceptor loop
        loop {
            let mut client = listener.accept().await;
            info!("New client !");

            #[cfg(feature = "trace")]
            let (trace_server, trace_data) = (trace_server.clone(), trace_data_c.clone());

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

                // Receive client trace data at the end of transmission
                // Only when using the postprocessor
                #[cfg(feature = "trace")]
                {
                    // Wait at most 3 seconds to receive client trace data
                    match timeout(Duration::from_secs(3), trace_server.accept()).await {
                        Ok(Ok((mut client, _))) => {
                            let mut buf = Vec::new();
                            client.read_to_end(&mut buf).await.unwrap();
                            let data: metrics::Data = bincode::deserialize(&buf).unwrap();
                            debug!("Received client trace data");
                            debug!("Transfer time : {} ms", data.execution_time.as_millis(),);
                            debug!("Total throughput : {:.3} Mo/s", data.throughput_mo);
                            *trace_data.lock().await = data;
                        }
                        Ok(Err(_)) => {
                            error!("Failed to accept client");
                        }
                        Err(_) => {
                            debug!("No client trace data");
                        }
                    }
                }
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
        use plotly::common::{AxisSide, Line, LineShape, Marker, MarkerSymbol, Mode, Title};
        use plotly::configuration::{ImageButtonFormats, ToImageButtonOptions};
        use plotly::layout::{Axis, GridPattern, LayoutGrid, RangeSlider, RowOrder};
        use plotly::Configuration;
        use plotly::{Layout, Plot, Scatter};

        // https://plotly.com/javascript/plotly-fundamentals/
        // https://igiagkiozis.github.io/plotly/content/recipes/subplots/multiple_axes.html

        let (sent, ack, ack_envelope, dup, to) = {
            let trace = trace_recorder.lock().unwrap();
            let mut envelope_t = Vec::with_capacity(trace.acks.len());
            let mut envelope = Vec::with_capacity(trace.acks.len());
            let mut max = 0;
            for (&ack, ts) in trace.acks.iter().zip(&trace.timestamps_ack) {
                if ack > max {
                    max = ack;
                    envelope.push(ack);
                    envelope_t.push(ts.clone());
                }
            }
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
                Scatter::new(envelope_t, envelope)
                    .mode(Mode::Lines)
                    .line(Line::new().shape(LineShape::Hv))
                    .web_gl_mode(true)
                    .name("ACK envelope"),
                Scatter::new(trace.timestamps_dup.clone(), trace.dup.clone())
                    .mode(Mode::Markers)
                    .marker(
                        Marker::new()
                            .symbol(MarkerSymbol::TriangleLeft)
                            .color("#00FF00"),
                    )
                    .web_gl_mode(true)
                    .name("Retransmit (dup acks)"),
                Scatter::new(trace.timestamps_to.clone(), trace.to.clone())
                    .mode(Mode::Markers)
                    .marker(
                        Marker::new()
                            .symbol(MarkerSymbol::TriangleLeft)
                            .color("#0000FF"),
                    )
                    .web_gl_mode(true)
                    .name("Retransmit (timeouts)"),
            )
        };

        /*
        let srtt = {
            let srtt = srtt_metric.lock().unwrap();
            Scatter::new(srtt.timestamps.clone(), srtt.values.clone())
                .mode(Mode::Lines)
                .web_gl_mode(true)
                .name("SRTT")
                .y_axis("y2")
        };*/

        let throughput = {
            let mut throughput = throughput_metric.lock().unwrap();
            throughput.timestamps.pop();
            throughput.values.pop();
            Scatter::new(throughput.timestamps.clone(), throughput.values.clone())
                .mode(Mode::Lines)
                .web_gl_mode(true)
                .line(Line::new().shape(LineShape::Hv))
                .name("Throughput")
                .y_axis("y2")
        };

        let (client_msg, limit_min, client_ack, client_drop) = {
            let data = trace_data.lock().await;
            let mut limit_min = Vec::with_capacity(data.msgs.len());
            let mut limit_min_t = Vec::with_capacity(data.msgs.len());
            let mut max = 0;
            for (x, &y) in data.timestamps_msg.iter().zip(&data.msgs) {
                if y > max {
                    max = y;
                    limit_min.push(y);
                    limit_min_t.push(
                        x.checked_add_signed(chrono::Duration::microseconds(1000))
                            .unwrap(),
                    );
                }
            }
            (
                Scatter::new(data.timestamps_msg.clone(), data.msgs.clone())
                    .mode(Mode::Markers)
                    .web_gl_mode(true)
                    .marker(Marker::new().symbol(MarkerSymbol::Cross))
                    .name("(Client) Received"),
                Scatter::new(limit_min_t, limit_min)
                    .mode(Mode::Lines)
                    .web_gl_mode(true)
                    .name("ACK limit min"),
                Scatter::new(data.timestamps_ack.clone(), data.acks.clone())
                    .mode(Mode::Markers)
                    .web_gl_mode(true)
                    .marker(Marker::new().symbol(MarkerSymbol::AsteriskOpen))
                    .name("(Client) ACK sent"),
                Scatter::new(data.timestamps_drop.clone(), data.drops.clone())
                    .mode(Mode::Markers)
                    .marker(
                        Marker::new()
                            .symbol(MarkerSymbol::TriangleUp)
                            .color("#FF0000"),
                    )
                    .web_gl_mode(true)
                    .name("(Client) lost"),
            )
        };

        // Plot
        let mut plot = Plot::new();
        plot.add_traces(vec![sent, ack, ack_envelope, dup, to]);
        plot.add_traces(vec![client_msg, limit_min, client_ack, client_drop]);
        //plot.add_trace(srtt);
        plot.add_trace(throughput);

        plot.set_layout(
            Layout::new()
                .grid(
                    LayoutGrid::new()
                        .rows(2)
                        .columns(1)
                        .pattern(GridPattern::Coupled)
                        .row_order(RowOrder::TopToBottom),
                )
                .y_axis(Axis::new().title(Title::new("sequence number")))
                /*
                .y_axis2(
                    Axis::new()
                        .title(Title::new("SRTT (µs)"))
                        .side(AxisSide::Right)
                        .domain(&[0., 5000.]),
                )*/
                .y_axis2(
                    Axis::new()
                        .title(Title::new("Throughput (Mo/s)"))
                        //.overlaying("y")
                        .side(AxisSide::Left)
                        .domain(&[0., 20.]),
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
