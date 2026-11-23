//! When the "Up next" card appears (as the credits start, rather than after the file ends),
//! and which chapters get a "Skip Intro" / "Skip Recap" button.

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

/// What a skip button skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Intro,
    Recap,
}

impl Skip {
    pub fn label(self) -> &'static str {
        match self {
            Self::Intro => "Skip Intro",
            Self::Recap => "Skip Recap",
        }
    }
}

/// Skippable chapters are short and early: an intro longer than this, or starting later than
/// this share of the file, is more likely a mislabelled chapter.
const SKIP_MAX_LEN: f64 = 5. * 60.;
const SKIP_LATEST: f64 = 0.4;

/// Whether chapter `index` is an intro or recap, and where it ends (the next chapter's start,
/// else the end of the file).
pub fn skippable(chapters: &[Chapter], index: usize, duration: f64) -> Option<(Skip, f64)> {
    let chapter = chapters.get(index)?;
    let title = chapter.title.as_deref()?.to_lowercase();
    let words: Vec<&str> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.iter().any(|word| word.starts_with(w));
    let kind = if has("recap") || has("previously") || title.contains("last time") {
        Skip::Recap
    } else if has("intro")
        || has("opening")
        || words.contains(&"op")
        || title.contains("title sequence")
        || title.contains("main title")
        || title.contains("theme song")
    {
        Skip::Intro
    } else {
        return None;
    };
    let end = chapters.get(index + 1).map_or(duration, |next| next.time);
    let fits = duration > 0.
        && chapter.time <= duration * SKIP_LATEST
        && end - chapter.time <= SKIP_MAX_LEN
        && end > chapter.time;
    fits.then_some((kind, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_intros_and_recaps() {
        let chapters = [
            chapter("Previously On", 0.),
            chapter("Opening Credits", 45.),
            chapter("Part 1", 130.),
            chapter("Intro to the finale", 2000.),
        ];
        assert_eq!(skippable(&chapters, 0, 2700.), Some((Skip::Recap, 45.)));
        assert_eq!(skippable(&chapters, 1, 2700.), Some((Skip::Intro, 130.)));
        assert_eq!(skippable(&chapters, 2, 2700.), None);
        // Late in the file: not an intro, whatever it's called.
        assert_eq!(skippable(&chapters, 3, 2700.), None);
        // "Opera" isn't "op"; "Scope" doesn't start with it.
        assert_eq!(
            skippable(&[chapter("Opera", 0.), chapter("Scope", 60.)], 0, 2700.),
            None
        );
    }

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
