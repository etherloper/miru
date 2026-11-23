//! Thin, typed wrapper around libmpv for Miru.
//!
//! mpv renders into a native window it is given (`wid`) using its own GPU swap chain and
//! hardware decoding. The UI layer only issues commands and reads playback state.

use std::path::Path;
use std::time::{Duration, Instant};

use libmpv2::Mpv;
use libmpv2::events::Event;

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame capture: {0}")]
    Capture(String),
}

pub type Result<T> = std::result::Result<T, PlayerError>;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(20);

/// Renders a single frame of `input`, taken `fraction` of the way in, to a JPEG at `output`,
/// scaled to `width` pixels wide. Runs a headless mpv instance; blocks until done.
pub fn capture_frame(input: &Path, output: &Path, fraction: f64, width: u32) -> Result<()> {
    let input = input
        .to_str()
        .ok_or_else(|| PlayerError::Capture("path is not UTF-8".into()))?;
    // mpv's image output writes numbered files into a directory; use a private one per capture.
    let staging = output.with_extension("frames");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let staging_str = staging
        .to_str()
        .ok_or_else(|| PlayerError::Capture("path is not UTF-8".into()))?;

    let result = (|| -> Result<()> {
        let mpv = Mpv::with_initializer(|init| {
            init.set_property("vo", "image")?;
            init.set_property("vo-image-format", "jpg")?;
            init.set_property("vo-image-jpeg-quality", 88i64)?;
            init.set_property("vo-image-outdir", staging_str)?;
            init.set_property("frames", 1i64)?;
            init.set_property("start", format!("{:.1}%", (fraction * 100.).clamp(0., 99.)))?;
            init.set_property("vf", format!("scale={width}:-2"))?;
            init.set_property("audio", "no")?;
            init.set_property("sid", "no")?;
            init.set_property("hwdec", "no")?;
            init.set_property("load-scripts", false)?;
            init.set_property("config", false)?;
            Ok(())
        })?;
        mpv.command("loadfile", &[input])?;
        let started = Instant::now();
        loop {
            let remaining = CAPTURE_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(PlayerError::Capture("timed out".into()));
            }
            match mpv.wait_event(remaining.as_secs_f64().min(1.0)) {
                Some(Ok(Event::EndFile(_))) => break,
                Some(Err(e)) => return Err(e.into()),
                _ => {}
            }
        }
        let frame = std::fs::read_dir(&staging)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("jpg")))
            .ok_or_else(|| PlayerError::Capture("no frame was produced".into()))?;
        std::fs::rename(frame, output)?;
        Ok(())
    })();
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        tracing::debug!("cleaning up {}: {e}", staging.display());
    }
    result
}

/// Channel layout sent to the audio device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioChannels {
    /// Downmix to stereo. Keeps centre-channel dialogue audible on headphones, TV speakers
    /// and virtual-surround drivers that would otherwise drop it.
    #[default]
    Stereo,
    /// The file's own layout, if the device accepts it.
    Source,
    /// Always 7.1 (upmixing 5.1), for headset virtualisers that expect a full 7.1 feed.
    Surround71,
}

impl AudioChannels {
    fn mpv_value(self) -> &'static str {
        match self {
            Self::Stereo => "stereo",
            Self::Source => "auto-safe",
            Self::Surround71 => "7.1",
        }
    }
}

/// Per-session preferences applied when mpv starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackOptions {
    pub subtitles: bool,
    pub audio_channels: AudioChannels,
    /// ISO 639 codes, e.g. "en".
    pub audio_language: Option<String>,
    pub subtitle_language: Option<String>,
    /// Starting volume, 0–100.
    pub volume: u32,
    /// Text subtitle size, as mpv's `sub-scale` in hundredths (100 = default).
    pub subtitle_scale_pct: u32,
    /// Specific tracks to start with (mpv track ids), for choices whose tracks have no
    /// language tag. Take precedence over the language preferences.
    pub audio_track: Option<i64>,
    pub subtitle_track: Option<i64>,
    /// Track titles to prefer among tracks in the chosen language (e.g. "SDH" over
    /// "Forced"), picked once the file's tracks are known.
    pub audio_title: Option<String>,
    pub subtitle_title: Option<String>,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self {
            subtitles: true,
            audio_channels: AudioChannels::Stereo,
            audio_language: None,
            subtitle_language: None,
            volume: 100,
            subtitle_scale_pct: 100,
            audio_track: None,
            subtitle_track: None,
            audio_title: None,
            subtitle_title: None,
        }
    }
}

/// A track to switch to once the file is loaded: same language and title.
#[derive(Debug, Clone)]
struct PreferredTrack {
    kind: TrackKind,
    lang: Option<String>,
    title: String,
}

/// Snapshot of playback state, polled by the UI.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlaybackState {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub idle: bool,
    /// Playback reached the end (the file stays open because of `keep-open`).
    pub eof: bool,
    /// 0–100.
    pub volume: f64,
    pub muted: bool,
    /// mpv's video output is up, so frames are reaching the window.
    pub video_ready: bool,
}

pub struct Player {
    mpv: Mpv,
    /// Track variants still to pick (see `apply_preferred_tracks`).
    preferred: std::sync::Mutex<Vec<PreferredTrack>>,
}

impl Player {
    /// Creates a player that renders into the native window `wid` (an HWND on Windows).
    pub fn new(wid: i64, options: &PlaybackOptions) -> Result<Self> {
        let mpv = Mpv::with_initializer(|init| {
            // Track preferences: mpv picks matching tracks itself when a file loads.
            if let Some(lang) = &options.audio_language {
                init.set_property("alang", lang.as_str())?;
            }
            if let Some(lang) = &options.subtitle_language {
                init.set_property("slang", lang.as_str())?;
            }
            match (options.subtitles, options.subtitle_track) {
                (false, _) => init.set_property("sid", "no")?,
                (true, Some(id)) => init.set_property("sid", id)?,
                (true, None) => init.set_property("sid", "auto")?,
            }
            if let Some(id) = options.audio_track {
                init.set_property("aid", id)?;
            }
            init.set_property("audio-channels", options.audio_channels.mpv_value())?;
            init.set_property("volume", i64::from(options.volume.min(100)))?;
            init.set_property("sub-scale", f64::from(options.subtitle_scale_pct) / 100.0)?;
            // mpv buffers up to ~200 MB by default, sized for network streams; local files only
            // need a few seconds of read-ahead and a short seek-back window.
            init.set_property("demuxer-max-bytes", "64MiB")?;
            init.set_property("demuxer-max-back-bytes", "16MiB")?;
            init.set_property("wid", wid)?;
            init.set_property("vo", "gpu-next")?;
            init.set_property("hwdec", "auto-safe")?;
            init.set_property("keep-open", "yes")?;
            // The UI owns all input and on-screen controls.
            init.set_property("input-default-bindings", false)?;
            init.set_property("input-vo-keyboard", false)?;
            init.set_property("input-cursor", false)?;
            init.set_property("cursor-autohide", "no")?;
            init.set_property("osc", false)?;
            init.set_property("osd-level", 0i64)?;
            Ok(())
        })?;
        let mut preferred = Vec::new();
        if options.audio_track.is_none()
            && let Some(title) = &options.audio_title
        {
            preferred.push(PreferredTrack {
                kind: TrackKind::Audio,
                lang: options.audio_language.clone(),
                title: title.clone(),
            });
        }
        if options.subtitles
            && options.subtitle_track.is_none()
            && let Some(title) = &options.subtitle_title
        {
            preferred.push(PreferredTrack {
                kind: TrackKind::Subtitle,
                lang: options.subtitle_language.clone(),
                title: title.clone(),
            });
        }
        Ok(Self {
            mpv,
            preferred: std::sync::Mutex::new(preferred),
        })
    }

    /// Once the file's tracks are known, switches to the preferred variants: mpv picks by
    /// language alone, so with "English" and "English SDH" it may pick the wrong one. Cheap to
    /// call repeatedly; does nothing after the first time it finds tracks.
    pub fn apply_preferred_tracks(&self) {
        let Ok(mut preferred) = self.preferred.lock() else {
            return;
        };
        if preferred.is_empty() {
            return;
        }
        let count: i64 = self.mpv.get_property("track-list/count").unwrap_or(0);
        if count == 0 {
            return;
        }
        let tracks = self.tracks();
        for want in preferred.drain(..) {
            let found = tracks.iter().find(|t| {
                t.kind == want.kind
                    && t.lang == want.lang
                    && t.title.as_deref() == Some(want.title.as_str())
            });
            let Some(track) = found.filter(|t| !t.selected) else {
                continue;
            };
            let result = match want.kind {
                TrackKind::Audio => self.select_audio(track.id),
                TrackKind::Subtitle => self.select_subtitle(Some(track.id)),
            };
            if let Err(e) = result {
                tracing::warn!("cannot select preferred track: {e}");
            }
        }
    }

    /// Loads a file, optionally starting at `start` seconds (for resume).
    pub fn load(&self, path: &str, start: Option<f64>) -> Result<()> {
        match start {
            // `loadfile <url> <flags> <index> <options>`; index -1 means "no playlist index".
            Some(start) => {
                let options = format!("start={start:.3}");
                Ok(self
                    .mpv
                    .command("loadfile", &[path, "replace", "-1", &options])?)
            }
            None => Ok(self.mpv.command("loadfile", &[path, "replace"])?),
        }
    }

    /// Audio and subtitle tracks of the current file, in mpv's order.
    pub fn tracks(&self) -> Vec<Track> {
        let count: i64 = self.mpv.get_property("track-list/count").unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                let prop = |name: &str| format!("track-list/{i}/{name}");
                let kind = match self
                    .mpv
                    .get_property::<String>(&prop("type"))
                    .ok()?
                    .as_str()
                {
                    "audio" => TrackKind::Audio,
                    "sub" => TrackKind::Subtitle,
                    _ => return None,
                };
                let text = |name: &str| {
                    self.mpv
                        .get_property::<String>(&prop(name))
                        .ok()
                        .filter(|s| !s.is_empty())
                };
                Some(Track {
                    id: self.mpv.get_property(&prop("id")).ok()?,
                    kind,
                    title: text("title"),
                    lang: text("lang"),
                    codec: text("codec"),
                    channels: self.mpv.get_property(&prop("demux-channel-count")).ok(),
                    selected: self.mpv.get_property(&prop("selected")).unwrap_or(false),
                    external: self.mpv.get_property(&prop("external")).unwrap_or(false),
                })
            })
            .collect()
    }

    /// Chapters of the current file, in order.
    pub fn chapters(&self) -> Vec<Chapter> {
        let count: i64 = self.mpv.get_property("chapter-list/count").unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                let time: f64 = self
                    .mpv
                    .get_property(&format!("chapter-list/{i}/time"))
                    .ok()?;
                let title = self
                    .mpv
                    .get_property::<String>(&format!("chapter-list/{i}/title"))
                    .ok()
                    .filter(|t| !t.trim().is_empty());
                Some(Chapter { title, time })
            })
            .collect()
    }

    /// Index of the chapter being played, if the file has chapters.
    pub fn current_chapter(&self) -> Option<usize> {
        self.mpv
            .get_property::<i64>("chapter")
            .ok()
            .and_then(|c| usize::try_from(c).ok())
    }

    pub fn seek_chapter(&self, index: usize) -> Result<()> {
        Ok(self.mpv.set_property("chapter", index as i64)?)
    }

    /// Technical details of the playing file, for the player's Media info page.
    pub fn media_info(&self) -> MediaInfo {
        let text = |name: &str| {
            self.mpv
                .get_property::<String>(name)
                .ok()
                .filter(|s| !s.is_empty())
        };
        let int = |name: &str| self.mpv.get_property::<i64>(name).ok().filter(|v| *v > 0);
        let float = |name: &str| self.mpv.get_property::<f64>(name).ok().filter(|v| *v > 0.);
        MediaInfo {
            path: text("path"),
            container: text("file-format"),
            size_bytes: int("file-size"),
            video_codec: text("video-format"),
            width: int("video-params/w"),
            height: int("video-params/h"),
            fps: float("container-fps").or_else(|| float("estimated-vf-fps")),
            gamma: text("video-params/gamma"),
            video_bitrate: float("video-bitrate"),
            audio_codec: text("audio-codec-name"),
            audio_channels: text("audio-params/hr-channels"),
            sample_rate: int("audio-params/samplerate"),
            audio_bitrate: float("audio-bitrate"),
            hwdec: text("hwdec-current").filter(|h| h != "no"),
        }
    }

    /// The channel layout actually sent to the audio device, e.g. "stereo" or "5.1".
    pub fn audio_output_layout(&self) -> Option<String> {
        self.mpv
            .get_property::<String>("audio-out-params/hr-channels")
            .ok()
            .filter(|s| !s.is_empty())
    }

    pub fn select_audio(&self, id: i64) -> Result<()> {
        Ok(self.mpv.set_property("aid", id)?)
    }

    /// Selects a subtitle track, or turns subtitles off with `None`.
    pub fn select_subtitle(&self, id: Option<i64>) -> Result<()> {
        match id {
            Some(id) => Ok(self.mpv.set_property("sid", id)?),
            None => Ok(self.mpv.set_property("sid", "no")?),
        }
    }

    /// Raises subtitles above the on-screen controls while they are shown.
    /// `sub-margin-y` is in units of a 720-line-high script, independent of window size.
    pub fn lift_subtitles(&self, lifted: bool) -> Result<()> {
        Ok(self
            .mpv
            .set_property("sub-margin-y", if lifted { 110i64 } else { 22i64 })?)
    }

    /// Sets the volume (0–100); setting it also unmutes.
    pub fn set_volume(&self, volume: f64) -> Result<()> {
        self.mpv.set_property("volume", volume.clamp(0.0, 100.0))?;
        Ok(self.mpv.set_property("mute", false)?)
    }

    pub fn toggle_mute(&self) -> Result<()> {
        Ok(self.mpv.command("cycle", &["mute"])?)
    }

    pub fn toggle_pause(&self) -> Result<()> {
        Ok(self.mpv.command("cycle", &["pause"])?)
    }

    pub fn seek_relative(&self, seconds: f64) -> Result<()> {
        Ok(self
            .mpv
            .command("seek", &[&seconds.to_string(), "relative"])?)
    }

    pub fn seek_absolute(&self, seconds: f64) -> Result<()> {
        Ok(self
            .mpv
            .command("seek", &[&seconds.to_string(), "absolute"])?)
    }

    pub fn state(&self) -> PlaybackState {
        // Properties are unavailable while nothing is loaded; treat that as zero/idle.
        PlaybackState {
            position: self.mpv.get_property("time-pos").unwrap_or(0.0),
            duration: self.mpv.get_property("duration").unwrap_or(0.0),
            paused: self.mpv.get_property("pause").unwrap_or(false),
            idle: self.mpv.get_property("idle-active").unwrap_or(true),
            eof: self.mpv.get_property("eof-reached").unwrap_or(false),
            volume: self.mpv.get_property("volume").unwrap_or(100.0),
            muted: self.mpv.get_property("mute").unwrap_or(false),
            video_ready: self.mpv.get_property("vo-configured").unwrap_or(false),
        }
    }
}

/// What mpv reports about the playing file. Any field may be missing (bitrates are estimates
/// that appear after a few seconds of playback).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    pub path: Option<String>,
    /// mpv's demuxer name, e.g. "mkv" or "mp4".
    pub container: Option<String>,
    pub size_bytes: Option<i64>,
    /// Short codec name, e.g. "hevc" or "h264".
    pub video_codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fps: Option<f64>,
    /// Transfer function: "pq" (HDR10/Dolby Vision), "hlg", or an SDR curve.
    pub gamma: Option<String>,
    /// Bits per second.
    pub video_bitrate: Option<f64>,
    pub audio_codec: Option<String>,
    /// e.g. "5.1" or "stereo".
    pub audio_channels: Option<String>,
    pub sample_rate: Option<i64>,
    pub audio_bitrate: Option<f64>,
    /// The hardware decoder in use, if any (e.g. "d3d11va").
    pub hwdec: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub title: Option<String>,
    /// Start time in seconds.
    pub time: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Subtitle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: i64,
    pub kind: TrackKind,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
    pub channels: Option<i64>,
    pub selected: bool,
    pub external: bool,
}

impl Track {
    /// Main line for a track menu: the language, else the track title, else "Track N".
    pub fn primary(&self) -> String {
        self.lang
            .as_deref()
            .map(language_name)
            .or_else(|| self.title.clone())
            .unwrap_or_else(|| format!("Track {}", self.id))
    }

    /// Secondary line: title (if not already the primary), channel layout, format, external.
    pub fn detail(&self) -> String {
        let primary = self.primary();
        let mut parts: Vec<String> = Vec::new();
        if let Some(title) = &self.title
            && !title.eq_ignore_ascii_case(&primary)
        {
            parts.push(title.clone());
        }
        if self.kind == TrackKind::Audio
            && let Some(ch) = self.channels
        {
            parts.push(channel_label(ch));
        }
        if let Some(codec) = &self.codec {
            parts.push(codec_label(codec));
        }
        if self.external {
            parts.push("External".into());
        }
        parts.join(" · ")
    }

    /// Human-readable label, e.g. "English · Commentary · 5.1" or "Japanese (SRT)".
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(lang) = &self.lang {
            parts.push(language_name(lang));
        }
        if let Some(title) = &self.title
            && !parts.iter().any(|p| p.eq_ignore_ascii_case(title))
        {
            parts.push(title.clone());
        }
        if self.kind == TrackKind::Audio
            && let Some(ch) = self.channels
        {
            parts.push(match ch {
                1 => "Mono".into(),
                2 => "Stereo".into(),
                6 => "5.1".into(),
                8 => "7.1".into(),
                n => format!("{n} ch"),
            });
        }
        if parts.is_empty() {
            parts.push(format!("Track {}", self.id));
        }
        let mut label = parts.join(" · ");
        if self.kind == TrackKind::Subtitle
            && let Some(codec) = &self.codec
        {
            let codec = match codec.as_str() {
                "subrip" => "SRT",
                "ass" | "ssa" => "ASS",
                "hdmv_pgs_subtitle" => "PGS",
                "dvd_subtitle" => "VobSub",
                "webvtt" => "VTT",
                other => other,
            };
            label.push_str(&format!(" ({codec})"));
        }
        label
    }
}

fn channel_label(channels: i64) -> String {
    match channels {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n} ch"),
    }
}

fn codec_label(codec: &str) -> String {
    match codec {
        "subrip" => "SRT",
        "ass" | "ssa" => "ASS",
        "hdmv_pgs_subtitle" => "PGS",
        "dvd_subtitle" => "VobSub",
        "webvtt" => "VTT",
        "aac" => "AAC",
        "ac3" => "Dolby Digital",
        "eac3" => "Dolby Digital Plus",
        "truehd" => "Dolby TrueHD",
        "dts" => "DTS",
        "flac" => "FLAC",
        "opus" => "Opus",
        other => return other.to_uppercase(),
    }
    .to_string()
}

/// English name for common ISO 639-1/639-2 codes; unknown codes are shown uppercased.
fn language_name(code: &str) -> String {
    let name = match code.to_ascii_lowercase().as_str() {
        "en" | "eng" => "English",
        "es" | "spa" => "Spanish",
        "fr" | "fre" | "fra" => "French",
        "de" | "ger" | "deu" => "German",
        "it" | "ita" => "Italian",
        "pt" | "por" => "Portuguese",
        "ja" | "jpn" => "Japanese",
        "ko" | "kor" => "Korean",
        "zh" | "chi" | "zho" => "Chinese",
        "ru" | "rus" => "Russian",
        "nl" | "dut" | "nld" => "Dutch",
        "sv" | "swe" => "Swedish",
        "no" | "nor" => "Norwegian",
        "da" | "dan" => "Danish",
        "fi" | "fin" => "Finnish",
        "pl" | "pol" => "Polish",
        "tr" | "tur" => "Turkish",
        "ar" | "ara" => "Arabic",
        "hi" | "hin" => "Hindi",
        "und" => "Unknown",
        _ => return code.to_uppercase(),
    };
    name.to_string()
}

/// Formats seconds as `h:mm:ss` or `m:ss`.
pub fn format_time(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_labels() {
        let audio = Track {
            id: 2,
            kind: TrackKind::Audio,
            title: Some("Commentary".into()),
            lang: Some("eng".into()),
            codec: Some("aac".into()),
            channels: Some(6),
            selected: false,
            external: false,
        };
        assert_eq!(audio.label(), "English · Commentary · 5.1");
        let sub = Track {
            id: 1,
            kind: TrackKind::Subtitle,
            title: Some("English".into()),
            lang: Some("en".into()),
            codec: Some("subrip".into()),
            channels: None,
            ..audio.clone()
        };
        assert_eq!(sub.label(), "English (SRT)");
        assert_eq!(audio.primary(), "English");
        assert_eq!(audio.detail(), "Commentary · 5.1 · AAC");
        assert_eq!(sub.primary(), "English");
        assert_eq!(sub.detail(), "SRT");
        let bare = Track {
            title: None,
            lang: None,
            codec: None,
            channels: None,
            ..sub
        };
        assert_eq!(bare.label(), "Track 1");
        assert_eq!(language_name("xyz"), "XYZ");
    }

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(65.9), "1:05");
        assert_eq!(format_time(3725.0), "1:02:05");
        assert_eq!(format_time(f64::NAN), "0:00");
        assert_eq!(format_time(-3.0), "0:00");
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    /// Needs libmpv at runtime and the test clip; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn captures_a_frame() {
        let clip = std::env::var("MIRU_TEST_CLIP").expect("set MIRU_TEST_CLIP");
        let dir = std::env::temp_dir().join("miru-capture-test");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("frame.jpg");
        let _ = std::fs::remove_file(&out);
        capture_frame(Path::new(&clip), &out, 0.25, 480).unwrap();
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
    }
}

#[cfg(test)]
mod audio_probe {
    use super::*;

    /// Diagnostic: prints the audio layout mpv decodes vs. what it sends to the default device.
    /// Run with `MIRU_TEST_CLIP=<5.1 file> cargo test -p miru-player audio_probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_audio_output_layout() {
        let clip = std::env::var("MIRU_TEST_CLIP").expect("set MIRU_TEST_CLIP");
        for channels in ["auto-safe", "auto", "7.1,5.1,stereo", "stereo"] {
            let mpv = Mpv::with_initializer(|init| {
                init.set_property("vo", "null")?;
                init.set_property("volume", 0i64)?;
                init.set_property("aid", 2i64)?;
                init.set_property("audio-channels", channels)?;
                Ok(())
            })
            .unwrap();
            mpv.command("loadfile", &[&clip]).unwrap();
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(3) {
                let _ = mpv.wait_event(0.2);
            }
            let get = |p: &str| {
                mpv.get_property::<String>(p)
                    .unwrap_or_else(|e| format!("<{e}>"))
            };
            println!(
                "audio-channels={channels}: decoded={} -> output={} via {}",
                get("audio-params/hr-channels"),
                get("audio-out-params/hr-channels"),
                get("current-ao"),
            );
            println!(
                "  out layout: {}  device: {}",
                get("audio-out-params/channels"),
                get("audio-device")
            );
            if channels == "stereo" {
                println!("  devices: {}", get("audio-device-list"));
            }
        }
    }
}
