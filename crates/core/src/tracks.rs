//! Audio and subtitle choices remembered per show, so picking English subtitles on one
//! episode carries over to the next.

use rusqlite::{OptionalExtension, params};

use crate::db::{Db, Result};

/// What the viewer last chose for a show. `None` fields were never chosen and fall back to
/// the global settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackPrefs {
    pub audio_lang: Option<String>,
    /// mpv's audio track id, used when the chosen track has no language tag.
    pub audio_track: Option<i64>,
    pub subtitles_on: Option<bool>,
    pub subtitle_lang: Option<String>,
    pub subtitle_track: Option<i64>,
}

impl Db {
    pub fn show_track_prefs(&self, show: &str) -> Result<TrackPrefs> {
        Ok(self
            .conn
            .query_row(
                "SELECT audio_lang, audio_track, subtitles_on, subtitle_lang, subtitle_track
                 FROM show_track_prefs WHERE show = ?1",
                [show],
                |r| {
                    Ok(TrackPrefs {
                        audio_lang: r.get(0)?,
                        audio_track: r.get(1)?,
                        subtitles_on: r.get(2)?,
                        subtitle_lang: r.get(3)?,
                        subtitle_track: r.get(4)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Remembers the audio track picked for a show.
    pub fn set_show_audio(&self, show: &str, lang: Option<&str>, track: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO show_track_prefs (show, audio_lang, audio_track) VALUES (?1, ?2, ?3)
             ON CONFLICT(show) DO UPDATE SET
                audio_lang = excluded.audio_lang, audio_track = excluded.audio_track",
            params![show, lang, track],
        )?;
        Ok(())
    }

    /// Remembers the subtitle track picked for a show, or that subtitles were turned off
    /// (`track` is `None`).
    pub fn set_show_subtitles(
        &self,
        show: &str,
        lang: Option<&str>,
        track: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO show_track_prefs (show, subtitles_on, subtitle_lang, subtitle_track)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(show) DO UPDATE SET
                subtitles_on = excluded.subtitles_on,
                -- Turning subtitles off keeps the language for when they come back on.
                subtitle_lang = COALESCE(excluded.subtitle_lang, show_track_prefs.subtitle_lang),
                subtitle_track = COALESCE(excluded.subtitle_track, show_track_prefs.subtitle_track)",
            params![show, track.is_some(), lang, track],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_kept_per_show() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            db.show_track_prefs("Shogun").unwrap(),
            TrackPrefs::default()
        );

        db.set_show_audio("Shogun", Some("ja"), 2).unwrap();
        db.set_show_subtitles("Shogun", Some("en"), Some(3))
            .unwrap();
        db.set_show_subtitles("shogun", None, None).unwrap();
        let prefs = db.show_track_prefs("SHOGUN").unwrap();
        assert_eq!(prefs.audio_lang.as_deref(), Some("ja"));
        assert_eq!(prefs.audio_track, Some(2));
        assert_eq!(prefs.subtitles_on, Some(false));
        assert_eq!(prefs.subtitle_lang.as_deref(), Some("en"), "kept while off");

        assert_eq!(db.show_track_prefs("Dark").unwrap(), TrackPrefs::default());
    }
}
