//! Playback sleep timer. Deadlines use monotonic time and are not persisted.
use std::time::{Duration, Instant};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SleepTimer {
    #[default]
    Off,
    EndOfTrack,
    Deadline(Instant),
}
impl SleepTimer {
    pub fn after(now: Instant, duration: Duration) -> Self {
        Self::Deadline(now + duration)
    }
    pub fn expired(self, now: Instant) -> bool {
        matches!(self, Self::Deadline(deadline) if now >= deadline)
    }
    pub fn remaining(self, now: Instant) -> Option<Duration> {
        match self {
            Self::Deadline(deadline) => Some(deadline.saturating_duration_since(now)),
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_and_end_of_track_are_distinct() {
        let now = Instant::now();
        let timer = SleepTimer::after(now, Duration::from_secs(10));
        assert!(!timer.expired(now + Duration::from_secs(9)));
        assert!(timer.expired(now + Duration::from_secs(10)));
        assert_eq!(
            timer.remaining(now + Duration::from_secs(20)),
            Some(Duration::ZERO)
        );
        assert!(!SleepTimer::EndOfTrack.expired(now + Duration::from_secs(100)));
    }
}
