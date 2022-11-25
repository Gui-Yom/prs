use chrono::Utc;
use std::cell::RefCell;
use std::fmt::Debug;
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

pub trait StatsRecorder {
    fn record_int(&mut self, _field: &Field, _value: i128) {}
    fn record_str(&mut self, _field: &Field, _value: &str) {}
}

/// Trait adapter
struct VisitWrapper<R>(Rc<RefCell<R>>);

unsafe impl<R> Send for VisitWrapper<R> {}

impl<R: StatsRecorder> Visit for VisitWrapper<R> {
    fn record_i64(&mut self, field: &Field, value: i64) {
        (*self.0).borrow_mut().record_int(field, value as i128);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        (*self.0).borrow_mut().record_int(field, value as i128);
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        (*self.0).borrow_mut().record_int(field, value);
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        (*self.0).borrow_mut().record_int(field, value as i128);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        StatsRecorder::record_str((*self.0).borrow_mut().deref_mut(), field, value);
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

pub struct StatsLayer<R>(Mutex<VisitWrapper<R>>);

impl<R> StatsLayer<R> {
    pub fn new(recorder: Rc<RefCell<R>>) -> Self {
        Self(Mutex::new(VisitWrapper(recorder)))
    }
}

impl<R: StatsRecorder + 'static, S: Subscriber> Layer<S> for StatsLayer<R> {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        event.record(&mut *self.0.lock().unwrap());
    }
}

pub struct ThroughtputRecorder {
    pub timestamps: Vec<chrono::DateTime<Utc>>,
    pub values: Vec<f32>,
}

impl Default for ThroughtputRecorder {
    fn default() -> Self {
        Self {
            timestamps: Vec::with_capacity(4096),
            values: Vec::with_capacity(4096),
        }
    }
}

impl StatsRecorder for ThroughtputRecorder {
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
                    *last /= 0.010 * 1024.0;
                }
                self.values.push(value as f32);
            } else {
                *self.values.last_mut().unwrap() += value as f32
            }
        }
    }
}

pub struct TraceRecorder {
    pub ack_timestamps: Vec<chrono::DateTime<Utc>>,
    pub acks: Vec<u32>,
    pub frame_timestamps: Vec<chrono::DateTime<Utc>>,
    pub frames: Vec<u32>,
    pub lost_timestamps: Vec<chrono::DateTime<Utc>>,
    pub losses: Vec<u32>,
}

impl Default for TraceRecorder {
    fn default() -> Self {
        Self {
            ack_timestamps: Vec::with_capacity(1 << 15),
            acks: Vec::with_capacity(1 << 15),
            frame_timestamps: Vec::with_capacity(1 << 15),
            frames: Vec::with_capacity(1 << 15),
            lost_timestamps: Vec::with_capacity(1024),
            losses: Vec::with_capacity(1024),
        }
    }
}

impl StatsRecorder for TraceRecorder {
    fn record_int(&mut self, field: &Field, value: i128) {
        match field.name() {
            "rseq" => {
                self.ack_timestamps.push(Utc::now());
                self.acks.push(value as u32);
            }
            "lost" => {
                self.lost_timestamps.push(Utc::now());
                self.losses.push(value as u32);
            }
            _ => {}
        }
    }
}

pub struct SrttRecorder {
    pub timestamps: Vec<chrono::DateTime<Utc>>,
    pub values: Vec<u64>,
}

impl Default for SrttRecorder {
    fn default() -> Self {
        Self {
            timestamps: Vec::with_capacity(4096),
            values: Vec::with_capacity(4096),
        }
    }
}

impl StatsRecorder for SrttRecorder {
    fn record_int(&mut self, field: &Field, value: i128) {
        if "srtt" == field.name() {
            self.timestamps.push(Utc::now());
            self.values.push(value as u64);
        }
    }
}
