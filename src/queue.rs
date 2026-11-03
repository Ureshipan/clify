//! Queue operations are independent from the GUI and the playback backend.
use rand::{seq::SliceRandom, Rng};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}
impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Повтор: выкл",
            Self::All => "Повтор: все",
            Self::One => "Повтор: трек",
        }
    }
}
/// Select the next track after natural completion. Manual skipping remains linear.
pub fn next_after_completion(current: usize, len: usize, repeat: RepeatMode) -> Option<usize> {
    if current >= len {
        return None;
    }
    match repeat {
        RepeatMode::One => Some(current),
        _ if current + 1 < len => Some(current + 1),
        RepeatMode::All => Some(0),
        _ => None,
    }
}
/// Shuffle only upcoming entries; the current item and its history stay unchanged.
pub fn shuffle_upcoming<T, R: Rng + ?Sized>(items: &mut [T], current: usize, rng: &mut R) {
    if current < items.len() {
        items[current + 1..].shuffle(rng);
    }
}
/// Drop older history without changing the current item or upcoming entries.
pub fn trim_history<T>(items: &mut Vec<T>, current: &mut Option<usize>, keep: usize) -> usize {
    let Some(index) = *current else {
        return 0;
    };
    if index >= items.len() {
        return 0;
    }
    let removed = index.saturating_sub(keep);
    if removed > 0 {
        items.drain(..removed);
        *current = Some(index - removed);
    }
    removed
}
/// Remove an item without losing the identity of the currently playing entry.
pub fn remove<T>(items: &mut Vec<T>, current: &mut Option<usize>, index: usize) -> bool {
    if index >= items.len() || *current == Some(index) {
        return false;
    }
    items.remove(index);
    if let Some(pos) = current {
        if index < *pos {
            *pos -= 1;
        }
    }
    true
}
/// Swap adjacent entries, adjusting the current index when the current entry moves.
pub fn move_by<T>(
    items: &mut [T],
    current: &mut Option<usize>,
    index: usize,
    delta: isize,
) -> bool {
    if index >= items.len() || !matches!(delta, -1 | 1) {
        return false;
    }
    let Some(other) = index
        .checked_add_signed(delta)
        .filter(|other| *other < items.len())
    else {
        return false;
    };
    items.swap(index, other);
    if *current == Some(index) {
        *current = Some(other);
    } else if *current == Some(other) {
        *current = Some(index);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};
    #[test]
    fn repeat_ends_and_empty_queues() {
        assert_eq!(next_after_completion(0, 0, RepeatMode::All), None);
        assert_eq!(next_after_completion(1, 2, RepeatMode::Off), None);
        assert_eq!(next_after_completion(1, 2, RepeatMode::All), Some(0));
        assert_eq!(next_after_completion(1, 2, RepeatMode::One), Some(1));
        assert_eq!(next_after_completion(0, 2, RepeatMode::Off), Some(1));
    }
    #[test]
    fn shuffle_keeps_history_current_and_all_duplicates() {
        let mut items = vec!["a", "b", "c", "c", "d", "e"];
        let mut rng = StdRng::seed_from_u64(42);
        shuffle_upcoming(&mut items, 1, &mut rng);
        assert_eq!(&items[..2], &["a", "b"]);
        let mut remaining = items[2..].to_vec();
        remaining.sort();
        assert_eq!(remaining, vec!["c", "c", "d", "e"]);
    }
    #[test]
    fn history_trim_preserves_current_and_future() {
        let mut items = (0..500).collect::<Vec<_>>();
        let mut current = Some(350);
        assert_eq!(trim_history(&mut items, &mut current, 200), 150);
        assert_eq!(current, Some(200));
        assert_eq!(items[current.unwrap()], 350);
        assert_eq!(items.last(), Some(&499));
        assert_eq!(trim_history(&mut items, &mut current, 200), 0);
    }
    #[test]
    fn removing_and_moving_keep_current_identity() {
        let mut items = vec!["a", "b", "c", "d"];
        let mut current = Some(2);
        assert!(!remove(&mut items, &mut current, 2));
        assert!(remove(&mut items, &mut current, 0));
        assert_eq!(items[current.unwrap()], "c");
        assert!(move_by(&mut items, &mut current, 1, 1));
        assert_eq!(items[current.unwrap()], "c");
        assert!(!move_by(&mut items, &mut current, 2, 1));
    }
}
