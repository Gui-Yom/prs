use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::Mutex;
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

#[derive(Default)]
pub struct WindowStatsRecorder {
    pub values: Vec<u64>,
}

impl StatsRecorder for WindowStatsRecorder {
    fn record_int(&mut self, field: &Field, value: i128) {
        if "window_len" == field.name() {
            self.values.push(value as u64);
        }
    }
}
