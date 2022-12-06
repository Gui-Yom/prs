use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Trait to implement for a custom metric
pub trait Metric {
    fn record_int(&mut self, _field: &Field, _value: i128) {}
    fn record_str(&mut self, _field: &Field, _value: &str) {}
}

impl Visit for &mut dyn Metric {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_int(field, value as i128);
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_int(field, value as i128);
    }
    fn record_i128(&mut self, field: &Field, value: i128) {
        self.record_int(field, value as i128);
    }
    fn record_u128(&mut self, field: &Field, value: u128) {
        self.record_int(field, value as i128);
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

/// Tracing layer to record a metric
pub struct MetricLayer<R>(Arc<Mutex<R>>);

impl<R: Metric + Default> MetricLayer<R> {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(R::default())))
    }

    pub fn metric(&self) -> Arc<Mutex<R>> {
        self.0.clone()
    }
}

impl<R: Metric + 'static, S: Subscriber> Layer<S> for MetricLayer<R> {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Could probably be simplified
        event.record(&mut ((&mut *self.0.lock().unwrap()) as &mut dyn Metric) as &mut dyn Visit);
    }
}

pub struct ThroughputMetric {
    pub timestamps: Vec<DateTime<Utc>>,
    pub values: Vec<f32>,
}

impl Default for ThroughputMetric {
    fn default() -> Self {
        Self {
            timestamps: Vec::with_capacity(4096),
            values: Vec::with_capacity(4096),
        }
    }
}

impl Metric for ThroughputMetric {
    fn record_int(&mut self, field: &Field, value: i128) {
        if "sent" == field.name() {
            if self
                .timestamps
                .last()
                .map(|t| *t + chrono::Duration::milliseconds(10) < Utc::now())
                .unwrap_or(true)
            {
                self.timestamps.push(Utc::now());
                if let Some(last) = self.values.last_mut() {
                    *last /= 0.010 * 1000000.0;
                }
                self.values.push(value as f32);
            } else {
                *self.values.last_mut().unwrap() += value as f32
            }
        }
    }
}

/// Records the sequence number of packets sent and ack received
pub struct TraceRecorder {
    pub timestamps_ack: Vec<DateTime<Utc>>,
    pub acks: Vec<u32>,
    pub timestamps_sent: Vec<DateTime<Utc>>,
    pub sent: Vec<u32>,
    pub timestamps_anticip: Vec<DateTime<Utc>>,
    pub anticip: Vec<u32>,
    pub timestamps_to: Vec<DateTime<Utc>>,
    pub to: Vec<u32>,
}

impl Default for TraceRecorder {
    fn default() -> Self {
        Self {
            timestamps_ack: Vec::with_capacity(1 << 15),
            acks: Vec::with_capacity(1 << 15),
            timestamps_sent: Vec::with_capacity(1 << 15),
            sent: Vec::with_capacity(1 << 15),
            timestamps_anticip: Vec::with_capacity(1 << 12),
            anticip: Vec::with_capacity(1 << 12),
            timestamps_to: Vec::with_capacity(1 << 12),
            to: Vec::with_capacity(1 << 12),
        }
    }
}

impl Metric for TraceRecorder {
    fn record_int(&mut self, field: &Field, value: i128) {
        match field.name() {
            "seq" => {
                self.timestamps_sent.push(Utc::now());
                self.sent.push(value as u32);
            }
            "rseq" => {
                self.timestamps_ack.push(Utc::now());
                self.acks.push(value as u32);
            }
            "anticipation" => {
                self.timestamps_anticip.push(Utc::now());
                self.anticip.push(value as u32);
            }
            "timeout" => {
                self.timestamps_to.push(Utc::now());
                self.to.push(value as u32);
            }
            _ => {}
        }
    }
}

pub struct SrttMetric {
    pub timestamps: Vec<DateTime<Utc>>,
    pub values: Vec<u64>,
}

impl Default for SrttMetric {
    fn default() -> Self {
        Self {
            timestamps: Vec::with_capacity(4096),
            values: Vec::with_capacity(4096),
        }
    }
}

impl Metric for SrttMetric {
    fn record_int(&mut self, field: &Field, value: i128) {
        if "srtt" == field.name() {
            self.timestamps.push(Utc::now());
            self.values.push(value as u64);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Data {
    pub execution_time: Duration,
    pub throughput_mo: f32,
    pub timestamps_msg: Vec<DateTime<Utc>>,
    pub msgs: Vec<u32>,
    pub timestamps_drop: Vec<DateTime<Utc>>,
    pub drops: Vec<u32>,
    pub timestamps_ack: Vec<DateTime<Utc>>,
    pub acks: Vec<u32>,
}

impl Data {
    /// Const fn impl to use in statics
    pub const fn new() -> Self {
        Data {
            execution_time: Duration::ZERO,
            throughput_mo: 0.0,
            timestamps_msg: Vec::new(),
            msgs: Vec::new(),
            timestamps_drop: Vec::new(),
            drops: Vec::new(),
            timestamps_ack: Vec::new(),
            acks: Vec::new(),
        }
    }
}
