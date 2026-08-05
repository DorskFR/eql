use std::time::Duration;

pub const MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    current: Duration,
}

impl Backoff {
    pub fn new(base: Duration) -> Self {
        Self {
            base,
            max: MAX_BACKOFF,
            current: base,
        }
    }

    pub fn with_max(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            current: base,
        }
    }

    pub fn delay(&self) -> Duration {
        self.current
    }

    pub fn fail(&mut self) {
        self.current = (self.current * 2).min(self.max);
    }

    pub fn reset(&mut self) {
        self.current = self.base;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_up_to_the_cap_and_resets() {
        let mut backoff = Backoff::new(Duration::from_secs(5));
        assert_eq!(backoff.delay(), Duration::from_secs(5));

        let observed: Vec<u64> = (0..8)
            .map(|_| {
                backoff.fail();
                backoff.delay().as_secs()
            })
            .collect();
        assert_eq!(observed, vec![10, 20, 40, 80, 160, 300, 300, 300]);

        backoff.reset();
        assert_eq!(backoff.delay(), Duration::from_secs(5));
    }

    #[test]
    fn honours_a_custom_cap() {
        let mut backoff = Backoff::with_max(Duration::from_millis(10), Duration::from_millis(25));
        backoff.fail();
        assert_eq!(backoff.delay(), Duration::from_millis(20));
        backoff.fail();
        assert_eq!(backoff.delay(), Duration::from_millis(25));
    }
}
