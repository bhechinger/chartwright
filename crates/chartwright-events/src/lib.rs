use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    StepStarted {
        id: String,
        label: String,
        detail: Option<String>,
    },
    StepDetail {
        id: String,
        detail: String,
    },
    StepFinished {
        id: String,
        message: String,
        elapsed: Duration,
    },
    StepFailed {
        id: String,
        message: String,
    },
    Log {
        level: EventLevel,
        message: String,
    },
}

pub trait EventSink: Clone + Send + Sync + 'static {
    fn emit(&self, event: Event);
}

#[derive(Clone, Debug, Default)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: Event) {}
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryEventSink {
    events: Arc<Mutex<Vec<Event>>>,
}

impl InMemoryEventSink {
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .expect("event sink lock poisoned")
            .clone()
    }
}

impl EventSink for InMemoryEventSink {
    fn emit(&self, event: Event) {
        self.events
            .lock()
            .expect("event sink lock poisoned")
            .push(event);
    }
}

#[derive(Clone, Debug, Default)]
pub struct StderrEventSink;

impl EventSink for StderrEventSink {
    fn emit(&self, event: Event) {
        match event {
            Event::StepStarted { label, detail, .. } => {
                if let Some(detail) = detail {
                    eprintln!("info: {label}: {detail}");
                } else {
                    eprintln!("info: {label}: started");
                }
            }
            Event::StepDetail { detail, .. } => eprintln!("debug: {detail}"),
            Event::StepFinished {
                message, elapsed, ..
            } => eprintln!("info: {message} ({elapsed:?})"),
            Event::StepFailed { message, .. } => eprintln!("error: {message}"),
            Event::Log { level, message } => eprintln!("{}: {message}", level.as_str()),
        }
    }
}

impl EventLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}
