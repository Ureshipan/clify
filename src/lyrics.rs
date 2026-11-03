//! OpenSubsonic line-level lyrics and timing helpers.
use serde::Deserialize;
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    #[serde(default, deserialize_with = "crate::null_default")]
    pub value: String,
    #[serde(default)]
    pub start: Option<i64>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    #[serde(default, deserialize_with = "crate::null_default")]
    pub lang: String,
    #[serde(default, deserialize_with = "crate::null_default")]
    pub synced: bool,
    #[serde(default, deserialize_with = "crate::null_default")]
    pub offset: i64,
    #[serde(default, rename = "line", deserialize_with = "crate::null_default")]
    pub lines: Vec<LyricLine>,
}
impl Lyrics {
    /// OpenSubsonic offset is positive when lyrics should appear sooner.
    pub fn active_line(&self, position_ms: u64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let effective = position_ms as i128 + self.offset as i128;
        self.lines
            .iter()
            .rposition(|line| line.start.is_some_and(|start| start as i128 <= effective))
    }
    pub fn seek_position(&self, index: usize) -> Option<f64> {
        if !self.synced {
            return None;
        }
        let start = self.lines.get(index)?.start? as i128 - self.offset as i128;
        Some(start.max(0) as f64 / 1000.0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offset_and_unsynchronised_lyrics_are_handled() {
        let lines = vec![
            LyricLine {
                value: "a".into(),
                start: Some(1000),
            },
            LyricLine {
                value: "b".into(),
                start: Some(2500),
            },
        ];
        let lyrics = Lyrics {
            synced: true,
            offset: 200,
            lines,
            ..Default::default()
        };
        assert_eq!(lyrics.active_line(799), None);
        assert_eq!(lyrics.active_line(800), Some(0));
        assert_eq!(lyrics.active_line(2300), Some(1));
        assert_eq!(lyrics.seek_position(1), Some(2.3));
        assert_eq!(
            Lyrics {
                synced: false,
                ..lyrics
            }
            .active_line(9000),
            None
        );
    }
}
