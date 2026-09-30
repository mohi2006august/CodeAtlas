//! Retry policy for failed payments.
use crate::billing::math::clamp;
use std::{collections::HashMap, time::Duration};

mod metrics;

/// Decides when a payment is retried.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    max_attempts: u32,
}

pub trait Policy {
    fn next_delay(&self, attempt: u32) -> Duration;
}

impl RetryPolicy {
    /// Create a policy.
    pub fn new(max_attempts: u32) -> Self {
        RetryPolicy { max_attempts }
    }
}

impl Policy for RetryPolicy {
    fn next_delay(&self, attempt: u32) -> Duration {
        metrics::record("retry", attempt);
        Duration::from_millis(compute_delay(attempt))
    }
}

pub fn compute_delay(attempt: u32) -> u64 {
    clamp(500 * 2u64.pow(attempt), 0, 30_000)
}

pub enum Outcome {
    Retry,
    GiveUp,
}

macro_rules! retry {
    ($e:expr) => {
        $e
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_delay() {
        assert_eq!(compute_delay(20), 30_000);
    }
}
