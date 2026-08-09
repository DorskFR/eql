use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CAPACITY: usize = 4000;
pub const BATCH: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Batch {
    pub seq: u64,
    pub dropped: u64,
    pub lines: Vec<String>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[derive(Debug, Default)]
struct Inner {
    lines: VecDeque<String>,
    dropped: u64,
    next_seq: u64,
}

/// The daemon's own log between uploads. A daemon that cannot reach the server
/// must not grow without bound, so the oldest lines go first.
#[derive(Debug)]
pub struct Buffer {
    capacity: usize,
    inner: Mutex<Inner>,
}

impl Buffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    pub fn push(&self, line: String) {
        let mut inner = self.lock();
        while inner.lines.len() >= self.capacity {
            inner.lines.pop_front();
            inner.dropped += 1;
        }
        inner.lines.push_back(line);
    }

    pub fn len(&self) -> usize {
        self.lock().lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn take(&self, max: usize) -> Batch {
        let mut inner = self.lock();
        let count = inner.lines.len().min(max.max(1));
        if count == 0 {
            return Batch::default();
        }
        let lines: Vec<String> = inner.lines.drain(..count).collect();
        let seq = inner.next_seq;
        inner.next_seq += 1;
        let dropped = std::mem::take(&mut inner.dropped);
        Batch {
            seq,
            dropped,
            lines,
        }
    }

    /// Returns a failed batch to the front, and its sequence number with it, so
    /// the retry is the same upload rather than a new one the server must
    /// reconcile.
    pub fn put_back(&self, batch: Batch) {
        let mut inner = self.lock();
        inner.dropped += batch.dropped;
        for line in batch.lines.into_iter().rev() {
            if inner.lines.len() >= self.capacity {
                inner.dropped += 1;
                continue;
            }
            inner.lines.push_front(line);
        }
        inner.next_seq = inner.next_seq.min(batch.seq);
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

/// Unique per run without a uuid dependency: the start instant in nanoseconds
/// and the pid, which cannot repeat together on one machine.
pub fn session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    format!("{:x}-{:x}", nanos, std::process::id())
}

pub fn host_name() -> String {
    sysinfo::System::host_name()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
            return;
        }
        if !self.rest.is_empty() {
            self.rest.push(' ');
        }
        self.rest.push_str(&format!("{}={value:?}", field.name()));
    }
}

pub fn format_event(event: &tracing::Event<'_>, at: i64) -> String {
    let mut fields = Fields::default();
    event.record(&mut fields);
    let meta = event.metadata();
    let mut line = format!("{at} {:<5} {}", meta.level().as_str(), meta.target());
    if !fields.message.is_empty() {
        line.push(' ');
        line.push_str(&fields.message);
    }
    if !fields.rest.is_empty() {
        line.push(' ');
        line.push_str(&fields.rest);
    }
    line
}

pub struct Capture {
    buffer: Arc<Buffer>,
}

impl Capture {
    pub fn new(buffer: Arc<Buffer>) -> Self {
        Self { buffer }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.buffer.push(format_event(event, unix_now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(lines: &[&str]) -> Buffer {
        let buffer = Buffer::new(CAPACITY);
        for line in lines {
            buffer.push((*line).to_string());
        }
        buffer
    }

    #[test]
    fn lines_come_back_in_order_and_batches_number_themselves() {
        let buffer = buffer(&["a", "b", "c"]);
        let first = buffer.take(2);
        assert_eq!(first.seq, 0);
        assert_eq!(first.lines, ["a", "b"]);
        assert_eq!(buffer.len(), 1);

        let second = buffer.take(10);
        assert_eq!(second.seq, 1);
        assert_eq!(second.lines, ["c"]);
        assert!(buffer.take(10).is_empty());
    }

    #[test]
    fn the_oldest_lines_go_first_when_the_buffer_is_full() {
        let buffer = Buffer::new(3);
        for line in ["a", "b", "c", "d", "e"] {
            buffer.push(line.to_string());
        }
        let batch = buffer.take(10);
        assert_eq!(batch.lines, ["c", "d", "e"], "kept the newest");
        assert_eq!(batch.dropped, 2);
        assert_eq!(
            buffer.take(10).dropped,
            0,
            "the drop count is reported once"
        );
    }

    #[test]
    fn a_failed_batch_goes_back_where_it_came_from() {
        let buffer = buffer(&["a", "b", "c"]);
        let batch = buffer.take(2);
        buffer.push("d".to_string());
        buffer.put_back(batch);

        let retry = buffer.take(10);
        assert_eq!(retry.seq, 0, "the retry keeps the original sequence");
        assert_eq!(retry.lines, ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_put_back_batch_that_no_longer_fits_is_counted_not_grown_into() {
        let buffer = Buffer::new(2);
        let batch = Batch {
            seq: 0,
            dropped: 0,
            lines: vec!["a".into(), "b".into(), "c".into()],
        };
        buffer.put_back(batch);
        let back = buffer.take(10);
        assert_eq!(back.lines.len(), 2);
        assert_eq!(back.dropped, 1);
    }

    #[test]
    fn a_session_id_is_unique_per_call_and_url_safe() {
        let one = session_id();
        let two = session_id();
        assert_ne!(one, two);
        assert!(
            one.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "{one}"
        );
    }

    #[test]
    fn a_host_name_is_always_something() {
        assert!(!host_name().is_empty());
    }
}
