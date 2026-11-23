//! Audio and subtitle choices: the exact tracks per file (for resuming it), and the language
//! and track title per show, so picking English SDH subtitles on one episode carries over to
//! the next.

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
    /// Track titles, to pick the same variant among tracks sharing a language.
    pub audio_title: Option<String>,
    pub subtitle_title: Option<String>,
}

/// The tracks last used for one file (mpv track ids, which are stable for a file).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileTrackPrefs {
    pub audio_track: Option<i64>,
    pub subtitles_on: Option<bool>,
    pub subtitle_track: Option<i64>,
}

impl Db {
    pub fn show_track_prefs(&self, show: &str) -> Result<TrackPrefs> {
        Ok(self
            .conn
            .query_row(
                "SELECT audio_lang, audio_track, subtitles_on, subtitle_lang, subtitle_track,
                        audio_title, subtitle_title
                 FROM show_track_prefs WHERE show = ?1",
                [show],
                |r| {
                    Ok(TrackPrefs {
                        audio_lang: r.get(0)?,
                        audio_track: r.get(1)?,
                        subtitles_on: r.get(2)?,
                        subtitle_lang: r.get(3)?,
                        subtitle_track: r.get(4)?,
                        audio_title: r.get(5)?,
                        subtitle_title: r.get(6)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Remembers the audio track picked for a show.
    pub fn set_show_audio(
        &self,
        show: &str,
        lang: Option<&str>,
        title: Option<&str>,
        track: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO show_track_prefs (show, audio_lang, audio_title, audio_track)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(show) DO UPDATE SET
                audio_lang = excluded.audio_lang, audio_title = excluded.audio_title,
                audio_track = excluded.audio_track",
            params![show, lang, title, track],
        )?;
        Ok(())
    }

    /// Remembers the subtitle track picked for a show, or that subtitles were turned off
    /// (`track` is `None`).
    pub fn set_show_subtitles(
        &self,
        show: &str,
        lang: Option<&str>,
        title: Option<&str>,
        track: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO show_track_prefs
                (show, subtitles_on, subtitle_lang, subtitle_title, subtitle_track)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(show) DO UPDATE SET
                subtitles_on = excluded.subtitles_on,
                -- Turning subtitles off keeps the track for when they come back on.
                subtitle_lang = CASE WHEN excluded.subtitles_on THEN excluded.subtitle_lang
                    ELSE show_track_prefs.subtitle_lang END,
                subtitle_title = CASE WHEN excluded.subtitles_on THEN excluded.subtitle_title
                    ELSE show_track_prefs.subtitle_title END,
                subtitle_track = CASE WHEN excluded.subtitles_on THEN excluded.subtitle_track
                    ELSE show_track_prefs.subtitle_track END",
            params![show, track.is_some(), lang, title, track],
        )?;
        Ok(())
    }

    pub fn file_track_prefs(&self, file_id: i64) -> Result<FileTrackPrefs> {
        Ok(self
            .conn
            .query_row(
                "SELECT audio_track, subtitles_on, subtitle_track
                 FROM file_track_prefs WHERE file_id = ?1",
                [file_id],
                |r| {
                    Ok(FileTrackPrefs {
                        audio_track: r.get(0)?,
                        subtitles_on: r.get(1)?,
                        subtitle_track: r.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn set_file_audio(&self, file_id: i64, track: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_track_prefs (file_id, audio_track) VALUES (?1, ?2)
             ON CONFLICT(file_id) DO UPDATE SET audio_track = excluded.audio_track",
            params![file_id, track],
        )?;
        Ok(())
    }

    /// Remembers the subtitle track used for a file, or that subtitles were off (`None`).
    pub fn set_file_subtitles(&self, file_id: i64, track: Option<i64>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_track_prefs (file_id, subtitles_on, subtitle_track)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(file_id) DO UPDATE SET
                subtitles_on = excluded.subtitles_on,
                subtitle_track = COALESCE(excluded.subtitle_track, file_track_prefs.subtitle_track)",
            params![file_id, track.is_some(), track],
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

        db.set_show_audio("Shogun", Some("ja"), None, 2).unwrap();
        db.set_show_subtitles("Shogun", Some("en"), Some("SDH"), Some(3))
            .unwrap();
        db.set_show_subtitles("shogun", None, None, None).unwrap();
        let prefs = db.show_track_prefs("SHOGUN").unwrap();
        assert_eq!(prefs.audio_lang.as_deref(), Some("ja"));
        assert_eq!(prefs.audio_track, Some(2));
        assert_eq!(prefs.subtitles_on, Some(false));
        assert_eq!(prefs.subtitle_lang.as_deref(), Some("en"), "kept while off");
        assert_eq!(
            prefs.subtitle_title.as_deref(),
            Some("SDH"),
            "kept while off"
        );

        // A track without a title replaces the old title rather than inheriting it.
        db.set_show_subtitles("Shogun", Some("en"), None, Some(4))
            .unwrap();
        assert_eq!(db.show_track_prefs("Shogun").unwrap().subtitle_title, None);

        assert_eq!(db.show_track_prefs("Dark").unwrap(), TrackPrefs::default());
    }

    #[test]
    fn choices_are_kept_per_file() {
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root("/m").unwrap();
        let batch = db.batch().unwrap();
        let movie = crate::parse::ParsedMedia::Movie {
            title: "Heat".into(),
            year: Some(1995),
        };
        let fp = crate::db::Fingerprint { size: 1, mtime: 1 };
        batch
            .upsert_file(root.id, "/m/heat.mkv", fp, &movie)
            .unwrap();
        batch.commit().unwrap();
        let id = db.file_by_path("/m/heat.mkv").unwrap().unwrap().id;
        assert_eq!(db.file_track_prefs(id).unwrap(), FileTrackPrefs::default());
        db.set_file_subtitles(id, Some(5)).unwrap();
        db.set_file_audio(id, 2).unwrap();
        db.set_file_subtitles(id, None).unwrap();
        let prefs = db.file_track_prefs(id).unwrap();
        assert_eq!(prefs.subtitles_on, Some(false));
        assert_eq!(prefs.subtitle_track, Some(5), "kept while off");
        assert_eq!(prefs.audio_track, Some(2));
    }
}
