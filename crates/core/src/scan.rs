//! Walks library folders and syncs the database with what is on disk.
//!
//! Scans are incremental: files whose size and mtime are unchanged are not re-parsed, and
//! files that disappeared are removed. Each root is applied in a single transaction.

use std::collections::HashSet;
use std::path::Path;
use std::time::UNIX_EPOCH;

use walkdir::{DirEntry, WalkDir};

use crate::db::{self, Db, Fingerprint, LibraryRoot};
use crate::parse::{PARSER_VERSION, parse_path};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "webm", "ts", "m2ts", "mpg", "mpeg", "flv", "ogv",
];

/// Folders holding bonus material rather than library items (Plex/Jellyfin conventions).
const EXTRAS_DIRS: &[&str] = &[
    "extras",
    "featurettes",
    "behind the scenes",
    "deleted scenes",
    "interviews",
    "scenes",
    "shorts",
    "trailers",
    "other",
    "sample",
    "samples",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    pub added: u32,
    pub updated: u32,
    pub unchanged: u32,
    pub removed: u32,
    /// Entries that could not be read or have non-UTF-8 paths.
    pub errors: u32,
}

pub fn scan_root(db: &mut Db, root: &LibraryRoot) -> db::Result<ScanReport> {
    let root_path = Path::new(&root.path);
    if !root_path.is_dir() {
        // An unplugged drive must not wipe the library, so leave its entries untouched.
        tracing::warn!(root = %root.path, "library root is not reachable; skipping");
        return Ok(ScanReport {
            errors: 1,
            ..ScanReport::default()
        });
    }

    // After a parser upgrade every file is re-parsed, even if it hasn't changed on disk.
    let reparse = db.parser_version()? < PARSER_VERSION;
    let batch = db.batch()?;
    let mut known = batch.fingerprints(root.id)?;
    let mut seen = HashSet::new();
    let mut report = ScanReport::default();

    let walker = WalkDir::new(root_path)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_ignored_dir(e));
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!("scan error: {e}");
                report.errors += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_video(entry.path()) || is_sample(entry.path()) {
            continue;
        }
        let Some(path) = entry.path().to_str() else {
            tracing::warn!(path = ?entry.path(), "skipping non-UTF-8 path");
            report.errors += 1;
            continue;
        };
        let fingerprint = match fingerprint(&entry) {
            Ok(fp) => fp,
            Err(e) => {
                tracing::warn!(path, "cannot read metadata: {e}");
                report.errors += 1;
                continue;
            }
        };
        seen.insert(path.to_string());

        match known.remove(path) {
            Some((_, old)) if old == fingerprint && !reparse => report.unchanged += 1,
            existing => {
                // Parse relative to the root so folder names above the library are ignored.
                let relative = entry.path().strip_prefix(root_path).unwrap_or(entry.path());
                batch.upsert_file(root.id, path, fingerprint, &parse_path(relative))?;
                if existing.is_some() {
                    report.updated += 1;
                } else {
                    report.added += 1;
                }
            }
        }
    }

    // Whatever is left in `known` was not found on disk this time.
    let gone: Vec<i64> = known.into_values().map(|(id, _)| id).collect();
    report.removed = gone.len() as u32;
    batch.delete_files(&gone)?;
    batch.commit()?;

    tracing::info!(root = %root.path, ?report, "scan complete");
    Ok(report)
}

fn is_ignored_dir(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy().to_lowercase();
    name.starts_with('.') || name.starts_with('$') || EXTRAS_DIRS.contains(&name.as_str())
}

/// Whether a path has a video file extension.
pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

fn is_sample(path: &Path) -> bool {
    path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| {
        let s = s.to_lowercase();
        s == "sample"
            || s.ends_with("-sample")
            || s.ends_with(".sample")
            || s.starts_with("sample-")
    })
}

fn fingerprint(entry: &DirEntry) -> std::io::Result<Fingerprint> {
    let meta = entry.metadata()?;
    let mtime = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    Ok(Fingerprint {
        size: meta.len() as i64,
        mtime,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::parse::ParsedMedia;

    fn touch(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn scans_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let root_dir = dir.path();
        touch(
            root_dir,
            "Movies/Arrival (2016)/Arrival.2016.1080p.mkv",
            "a",
        );
        touch(
            root_dir,
            "Movies/Arrival (2016)/Featurettes/Making Of.mkv",
            "x",
        );
        touch(root_dir, "Movies/Heat.1995.mp4", "b");
        touch(root_dir, "Movies/Heat.1995-sample.mp4", "x");
        touch(root_dir, "TV/Dark/Season 1/Dark.S01E01.mkv", "c");
        touch(root_dir, "TV/Dark/Season 1/notes.txt", "x");
        touch(root_dir, ".hidden/Secret.2000.mkv", "x");

        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root(root_dir.to_str().unwrap()).unwrap();

        let first = scan_root(&mut db, &root).unwrap();
        // The app records the parser version once all roots are scanned.
        db.set_parser_version(PARSER_VERSION).unwrap();
        assert_eq!(
            first,
            ScanReport {
                added: 3,
                ..Default::default()
            }
        );
        let titles: Vec<_> = db
            .movies()
            .unwrap()
            .into_iter()
            .map(|m| (m.title, m.year))
            .collect();
        assert_eq!(
            titles,
            [("Arrival".into(), Some(2016)), ("Heat".into(), Some(1995))]
        );
        assert_eq!(db.shows().unwrap()[0].title, "Dark");

        let second = scan_root(&mut db, &root).unwrap();
        assert_eq!(
            second,
            ScanReport {
                unchanged: 3,
                ..Default::default()
            }
        );

        // Modify one file (size change), delete one, add one.
        touch(root_dir, "Movies/Heat.1995.mp4", "bigger");
        fs::remove_file(root_dir.join("TV/Dark/Season 1/Dark.S01E01.mkv")).unwrap();
        touch(root_dir, "TV/Dark/Season 1/Dark.S01E02.mkv", "d");
        let third = scan_root(&mut db, &root).unwrap();
        assert_eq!(
            third,
            ScanReport {
                added: 1,
                updated: 1,
                unchanged: 1,
                removed: 1,
                errors: 0
            }
        );

        let episodes = db.episodes("Dark").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(matches!(
            episodes[0].media,
            ParsedMedia::Episode { episode: 2, .. }
        ));
    }

    #[test]
    fn missing_root_keeps_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Heat.1995.mkv", "a");
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root(dir.path().to_str().unwrap()).unwrap();
        scan_root(&mut db, &root).unwrap();

        let gone = LibraryRoot {
            path: dir.path().join("unplugged").to_string_lossy().into(),
            ..root
        };
        let report = scan_root(&mut db, &gone).unwrap();
        assert_eq!(report.errors, 1);
        assert_eq!(db.movies().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod reparse_tests {
    use std::fs;

    use super::*;

    #[test]
    fn outdated_parser_version_reparses_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Heat.1995.mkv"), "a").unwrap();
        let mut db = Db::open_in_memory().unwrap();
        let root = db.add_root(dir.path().to_str().unwrap()).unwrap();
        scan_root(&mut db, &root).unwrap();
        db.set_parser_version(PARSER_VERSION).unwrap();
        assert_eq!(scan_root(&mut db, &root).unwrap().unchanged, 1);

        db.set_parser_version(PARSER_VERSION - 1).unwrap();
        let report = scan_root(&mut db, &root).unwrap();
        assert_eq!((report.updated, report.unchanged), (1, 0));
    }
}
