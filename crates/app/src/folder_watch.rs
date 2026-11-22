//! Watches the library folders so files added, removed or renamed while the app is open
//! show up without a restart. Changes are batched: a rescan is requested once the folder has
//! been quiet for `SETTLE`, so a large copy triggers one scan rather than hundreds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

const SETTLE: Duration = Duration::from_secs(3);

pub struct FolderWatch {
    debouncer: Debouncer<RecommendedWatcher>,
    watched: Vec<PathBuf>,
}

impl FolderWatch {
    /// Starts the watcher; `changed` receives a message whenever a rescan is worthwhile.
    pub fn start(
        changed: async_channel::Sender<()>,
    ) -> notify_debouncer_mini::notify::Result<Self> {
        let debouncer = new_debouncer(SETTLE, move |result: DebounceEventResult| match result {
            Ok(events) => {
                if events.iter().any(|e| relevant(&e.path)) {
                    // Full means a rescan is already queued; closed means the app is exiting.
                    let _ = changed.try_send(());
                }
            }
            Err(e) => tracing::warn!("folder watch error: {e}"),
        })?;
        Ok(Self {
            debouncer,
            watched: Vec::new(),
        })
    }

    /// Watches exactly `roots`, adding new folders and dropping removed ones. A folder that
    /// can't be watched (an unplugged drive) is logged and skipped; the next scan still
    /// covers it if it comes back.
    pub fn set_roots(&mut self, roots: &[PathBuf]) {
        let watcher = self.debouncer.watcher();
        for old in self.watched.iter().filter(|p| !roots.contains(p)) {
            if let Err(e) = watcher.unwatch(old) {
                tracing::debug!(path = %old.display(), "unwatching folder: {e}");
            }
        }
        let mut watched = Vec::new();
        for root in roots {
            if self.watched.contains(root) {
                watched.push(root.clone());
                continue;
            }
            match watcher.watch(root, RecursiveMode::Recursive) {
                Ok(()) => watched.push(root.clone()),
                Err(e) => tracing::warn!(path = %root.display(), "cannot watch folder: {e}"),
            }
        }
        self.watched = watched;
    }
}

/// Video files and folders. Other files (artwork, subtitles, partial downloads) don't change
/// the library. A path that no longer exists may have been a folder (with a dotted name such
/// as `Show.S01`), so it counts unless its extension says it was an ordinary file.
fn relevant(path: &Path) -> bool {
    const NOT_MEDIA: &[&str] = &[
        "jpg",
        "jpeg",
        "png",
        "webp",
        "nfo",
        "srt",
        "ass",
        "ssa",
        "sub",
        "idx",
        "txt",
        "part",
        "tmp",
        "!qb",
        "crdownload",
        "torrent",
    ];
    if miru_core::scan::is_video(path) || path.is_dir() {
        return true;
    }
    if path.exists() {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !NOT_MEDIA.contains(&e.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_videos_and_folders_matter() {
        assert!(relevant(Path::new("D:/tv/Show/S01E01.mkv")));
        assert!(relevant(Path::new("D:/tv/New Show")));
        assert!(!relevant(Path::new("D:/tv/Show/poster.jpg")));
        assert!(!relevant(Path::new("D:/dl/movie.mkv.!qB")));
    }
}
