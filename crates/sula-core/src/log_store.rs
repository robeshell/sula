//! In-app log ring buffer (UI-15). Mirrors Swift `LogStore` (max 500).

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;
use uuid::Uuid;

const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    pub fn from_tracing(level: &Level) -> Self {
        match *level {
            Level::ERROR => Self::Error,
            Level::WARN => Self::Warning,
            Level::INFO => Self::Info,
            Level::DEBUG | Level::TRACE => Self::Debug,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: String,
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

/// Receives log changes as they happen, so a shell can push them to its UI.
pub trait LogSink: Send + Sync {
    fn entry(&self, entry: &LogEntry);
    fn cleared(&self);
}

pub struct LogStore {
    entries: Mutex<VecDeque<LogEntry>>,
    sink: RwLock<Option<Arc<dyn LogSink>>>,
}

impl LogStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(VecDeque::with_capacity(128)),
            sink: RwLock::new(None),
        })
    }

    pub fn attach_sink(&self, sink: Arc<dyn LogSink>) {
        if let Ok(mut slot) = self.sink.write() {
            *slot = Some(sink);
        }
    }

    pub fn append(&self, level: LogLevel, message: impl Into<String>) {
        let entry = LogEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            message: message.into(),
        };
        if let Ok(mut guard) = self.entries.lock() {
            guard.push_back(entry.clone());
            while guard.len() > MAX_ENTRIES {
                guard.pop_front();
            }
        }
        if let Ok(sink) = self.sink.read() {
            if let Some(sink) = sink.as_ref() {
                sink.entry(&entry);
            }
        }
    }

    pub fn list(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.clear();
        }
        if let Ok(sink) = self.sink.read() {
            if let Some(sink) = sink.as_ref() {
                sink.cleared();
            }
        }
    }
}

pub struct AppLogLayer {
    store: Arc<LogStore>,
}

impl AppLogLayer {
    pub fn new(store: Arc<LogStore>) -> Self {
        Self { store }
    }
}

impl<S> Layer<S> for AppLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = if visitor.message.is_empty() {
            visitor.fields
        } else if visitor.fields.is_empty() {
            visitor.message
        } else {
            format!("{} {}", visitor.message, visitor.fields)
        };
        if message.is_empty() {
            return;
        }
        self.store
            .append(LogLevel::from_tracing(event.metadata().level()), message);
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.push_field(field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}").trim_matches('"').to_string();
        } else {
            self.push_field(field.name(), &format!("{value:?}"));
        }
    }
}

impl MessageVisitor {
    fn push_field(&mut self, name: &str, value: &str) {
        if name == "log.target" || name == "log.module_path" {
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(&mut self.fields, "{name}={value}");
    }
}
