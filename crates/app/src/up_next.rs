//! When the "Up next" card appears: as the credits start, rather than after the file ends.

use miru_player::Chapter;

/// Without a credits chapter, the card appears this long before the end.
const FALLBACK_LEAD: f64 = 30.;
/// Shorter files only get the card at the very end (a lead-in would cover too much of them).
const MIN_DURATION: f64 = 10. * 60.;
/// A credits chapter must start in the last quarter, so "Ending" in a chapter list of a
/// multi-part special doesn't trigger mid-episode.
const CREDITS_EARLIEST: f64 = 0.75;

/// Position (seconds) at which to offer the next episode: the start of a chapter named like
/// credits, else `FALLBACK_LEAD` before the end. `None` when the file is too short or its
/// length isn't known yet.
pub fn credits_start(duration: f64, chapters: &[Chapter]) -> Option<f64> {
    if duration < MIN_DURATION {
        return None;
    }
    let named_credits = chapters.iter().find(|c| {
        c.time >= duration * CREDITS_EARLIEST
            && c.title.as_deref().is_some_and(|t| {
                let t = t.to_lowercase();
                ["credit", "outro", "ending", "end"].iter().any(|w| {
                    t.split(|c: char| !c.is_alphanumeric())
                        .any(|part| part.starts_with(w))
                })
            })
    });
    Some(named_credits.map_or(duration - FALLBACK_LEAD, |c| c.time))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(title: &str, time: f64) -> Chapter {
        Chapter {
            title: Some(title.into()),
            time,
        }
    }

    #[test]
    fn uses_a_late_credits_chapter() {
        let chapters = [
            chapter("Intro", 0.),
            chapter("Episode", 90.),
            chapter("End Credits", 2580.),
        ];
        assert_eq!(credits_start(2700., &chapters), Some(2580.));
    }

    #[test]
    fn ignores_early_or_unrelated_chapters() {
        let chapters = [chapter("Ending of Part 1", 600.), chapter("Scene 9", 2600.)];
        assert_eq!(credits_start(2700., &chapters), Some(2670.));
        // "Friends" contains "end" but no word starts with it.
        assert_eq!(
            credits_start(2700., &[chapter("Friends", 2600.)]),
            Some(2670.)
        );
    }

    #[test]
    fn short_files_get_no_lead_in() {
        assert_eq!(credits_start(300., &[]), None);
        assert_eq!(credits_start(0., &[]), None);
    }
}
