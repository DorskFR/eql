/// A fault the daemon re-discovers on every tick, logged once and then quiet
/// until it changes or clears.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Notice {
    seen: Option<String>,
}

impl Notice {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when `message` differs from the last one reported.
    pub fn report(&mut self, message: impl Into<String>) -> bool {
        let message = message.into();
        if self.seen.as_deref() == Some(message.as_str()) {
            return false;
        }
        self.seen = Some(message);
        true
    }

    /// `true` when something had been reported and the condition is now gone,
    /// so the caller can say so once.
    pub fn clear(&mut self) -> bool {
        self.seen.take().is_some()
    }

    pub fn pending(&self) -> Option<&str> {
        self.seen.as_deref()
    }
}

/// [`Notice`], keyed by file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Notices {
    seen: std::collections::BTreeMap<String, String>,
}

impl Notices {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when `message` differs from the last one reported for `key`.
    pub fn report(&mut self, key: &str, message: impl Into<String>) -> bool {
        let message = message.into();
        if self.seen.get(key).is_some_and(|seen| *seen == message) {
            return false;
        }
        self.seen.insert(key.to_string(), message);
        true
    }

    /// `true` when `key` had been reported and its condition is now gone.
    pub fn clear(&mut self, key: &str) -> bool {
        self.seen.remove(key).is_some()
    }

    pub fn retain(&mut self, keep: &[String]) {
        self.seen.retain(|key, _| keep.iter().any(|k| k == key));
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_file_gets_its_own_one_line_report() {
        let mut notices = Notices::new();
        assert!(notices.is_empty());
        assert!(notices.report("a.json", "not json"));
        assert!(!notices.report("a.json", "not json"));
        assert!(notices.report("b.json", "not json"), "a different file");
        assert_eq!(notices.len(), 2);

        assert!(notices.report("a.json", "too large"), "a different fault");
        assert!(notices.clear("a.json"));
        assert!(!notices.clear("a.json"));

        notices.retain(&["c.json".to_string()]);
        assert!(notices.is_empty(), "b.json is gone from the directory");
        assert!(notices.report("b.json", "not json"), "and so is its report");
    }

    #[test]
    fn the_same_condition_is_reported_once_and_recovery_once() {
        let mut notice = Notice::new();
        assert_eq!(notice.pending(), None);
        assert!(notice.report("permission denied"));
        assert!(!notice.report("permission denied"), "still the same fault");
        assert_eq!(notice.pending(), Some("permission denied"));

        assert!(notice.report("no such file"), "a different fault speaks up");
        assert!(notice.clear(), "recovery is worth one line");
        assert!(!notice.clear(), "and only one");
        assert_eq!(notice.pending(), None);

        assert!(
            notice.report("no such file"),
            "the fault coming back is news again"
        );
    }
}
