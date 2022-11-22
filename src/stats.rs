use chrono::Utc;
use std::cell::RefCell;
use std::fmt::Debug;
use std::ops::Add;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

pub trait StatsRecorder {
    fn record_int(&mut self, _field: &Field, _value: i128) {}
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
    pub values: Vec<u32>,
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
                .map(|t| *t + chrono::Duration::milliseconds(50) < Utc::now())
                .unwrap_or(true)
            {
                self.timestamps.push(Utc::now());
                self.values.push(value as u32);
            } else {
                *self.values.last_mut().unwrap() += value as u32
            }
        }
    }
}
