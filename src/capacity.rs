use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Capacity {
    Bounded(usize),
    Unbounded(BackPressure),
}

impl Default for Capacity {
    fn default() -> Self {
        Capacity::Unbounded(BackPressure::default())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackPressure {
    pub start_at: usize,
    pub timeout: Duration,
    pub growth: Growth,
}

impl Default for BackPressure {
    fn default() -> Self {
        Self {
            start_at: 5,
            timeout: Duration::from_nanos(25),
            growth: Growth::Exponential(1.3),
        }
    }
}

impl BackPressure {
    /// Get a back-pressure configuration that is disabled.
    pub fn disabled() -> Self {
        Self {
            start_at: usize::MAX,
            timeout: Duration::from_nanos(0),
            growth: Growth::Linear,
        }
    }

    pub fn get_timeout(&self, msg_count: usize) -> Option<Duration> {
        if msg_count >= self.start_at {
            match self.growth {
                Growth::Exponential(factor) => {
                    let msg_count_diff = (msg_count - self.start_at).try_into().unwrap_or(i32::MAX);
                    let mult = factor.powi(msg_count_diff);
                    let nanos = self.timeout.as_nanos();
                    Some(Duration::from_nanos((nanos as f32 * mult) as u64))
                }
                Growth::Linear => {
                    let mult = (msg_count - self.start_at + 1) as u64;
                    let nanos = self.timeout.as_nanos();
                    Some(Duration::from_nanos(
                        (nanos * mult as u128).try_into().unwrap_or(u64::MAX),
                    ))
                }
            }
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Growth {
    /// `timeout = base_timeout * (growth ^ (msg_count - start_at))`
    Exponential(f32),
    /// `timeout = base_timeout * (msg_count - start_at)`
    Linear,
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;

    #[test]
    fn backpressure_linear() {
        let cfg = BackPressure {
            start_at: 0,
            timeout: Duration::from_secs(1),
            growth: Growth::Linear,
        };

        assert_eq!(cfg.get_timeout(0), Some(Duration::from_secs(1)));
        assert_eq!(cfg.get_timeout(1), Some(Duration::from_secs(2)));
        assert_eq!(cfg.get_timeout(10), Some(Duration::from_secs(11)));
    }

    #[test]
    fn backpressure_linear_start_at() {
        let cfg = BackPressure {
            start_at: 10,
            timeout: Duration::from_secs(1),
            growth: Growth::Linear,
        };

        assert_eq!(cfg.get_timeout(0), None);
        assert_eq!(cfg.get_timeout(1), None);
        assert_eq!(cfg.get_timeout(9), None);
        assert_eq!(cfg.get_timeout(10), Some(Duration::from_secs(1)));
        assert_eq!(cfg.get_timeout(11), Some(Duration::from_secs(2)));
        assert_eq!(cfg.get_timeout(20), Some(Duration::from_secs(11)));
    }

    #[test]
    fn backpressure_linear_max() {
        let cfg = BackPressure {
            start_at: usize::MAX,
            timeout: Duration::from_secs(1),
            growth: Growth::Linear,
        };

        assert_eq!(cfg.get_timeout(0), None);
        assert_eq!(cfg.get_timeout(1), None);
        assert_eq!(cfg.get_timeout(9), None);
        assert_eq!(cfg.get_timeout(usize::MAX - 1), None);
        assert_eq!(cfg.get_timeout(usize::MAX), Some(Duration::from_secs(1)));
    }

    #[test]
    fn backpressure_exponential() {
        let cfg = BackPressure {
            start_at: 0,
            timeout: Duration::from_secs(1),
            growth: Growth::Exponential(1.1),
        };

        assert_eq!(
            cfg.get_timeout(0),
            Some(Duration::from_nanos(1_000_000_000))
        );
        assert_eq!(
            cfg.get_timeout(1),
            Some(Duration::from_nanos(1_100_000_000))
        );
        assert_eq!(
            cfg.get_timeout(2),
            Some(Duration::from_nanos(1_210_000_000))
        );
        assert_eq!(
            cfg.get_timeout(3),
            Some(Duration::from_nanos(1_331_000_064))
        );
    }
}
