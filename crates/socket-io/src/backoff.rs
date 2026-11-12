//! Reconnect delay after the reference `contrib/backo2`: exponential, capped, with jitter.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

/// The reference `randomizationFactor`: how far each delay may stray either way, as a fraction of it.
const JITTER: f64 = 0.5;

pub struct Backoff {
    min: Duration,
    max: Duration,
    attempts: u32,
}

impl Backoff {
    /// Zero means the default, as in the reference `reconnectionDelay || 1000`.
    pub fn new(min: Duration, max: Duration) -> Backoff {
        let or = |value: Duration, default| if value.is_zero() { default } else { value };
        Backoff {
            min: or(min, Duration::from_secs(1)),
            max: or(max, Duration::from_secs(5)),
            attempts: 0,
        }
    }

    /// The delay before the next attempt, counting it.
    pub fn duration(&mut self) -> Duration {
        self.duration_with(random())
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
    }

    /// `duration` with a given random number in `0..1`.
    fn duration_with(&mut self, random: f64) -> Duration {
        let mut ms = self.min.as_secs_f64() * 1000.0 * 2f64.powi(self.attempts as i32);
        self.attempts = self.attempts.saturating_add(1);

        if ms.is_finite() {
            let deviation = (random * JITTER * ms).floor();
            // The reference picks the direction from the same random number.
            ms = if ((random * 10.0).floor() as u64) & 1 == 0 {
                ms - deviation
            } else {
                ms + deviation
            };
        }

        Duration::from_millis(ms.min(self.max.as_secs_f64() * 1000.0) as u64)
    }
}

/// A random number in `0..1`, from std's randomly keyed hasher.
fn random() -> f64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(0);
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_up_to_the_maximum() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(5));

        // No deviation with 0.
        let delays: Vec<u64> = (0..5)
            .map(|_| backoff.duration_with(0.0).as_millis() as u64)
            .collect();
        assert_eq!(delays, [1000, 2000, 4000, 5000, 5000]);

        backoff.reset();
        assert_eq!(backoff.duration_with(0.0), Duration::from_secs(1));
    }

    #[test]
    fn takes_the_reference_defaults_for_zero() {
        let mut backoff = Backoff::new(Duration::ZERO, Duration::ZERO);

        assert_eq!(backoff.duration_with(0.0), Duration::from_secs(1));
        backoff.attempts = 20;
        assert_eq!(backoff.duration_with(0.0), Duration::from_secs(5));
    }

    #[test]
    fn strays_by_the_jitter_in_the_reference_direction() {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(5));

        // floor(0.25 * 10) is even: shorter by floor(0.25 * 0.5 * 1000).
        assert_eq!(backoff.duration_with(0.25), Duration::from_millis(875));
        // floor(0.35 * 10) is odd: longer by floor(0.35 * 0.5 * 2000).
        assert_eq!(backoff.duration_with(0.35), Duration::from_millis(2350));
    }

    #[test]
    fn random_numbers_fall_in_range() {
        assert!((0..1000).map(|_| random()).all(|r| (0.0..1.0).contains(&r)));
    }
}
