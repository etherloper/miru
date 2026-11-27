//! Blocking library jobs (scan, metadata) run on background threads with their own
//! database connection; the UI reloads from its connection when they finish.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use miru_core::db::Db;
use miru_core::library::{ItemKind, LibraryItem};
use miru_core::scan::{ScanReport, scan_root};
use miru_metadata::{ArtworkCache, Tmdb, TmdbMatch, enrich_missing};

#[derive(Clone)]
pub struct AppPaths {
    pub db: PathBuf,
    pub artwork: PathBuf,
}

/// The app was called Nocturne before. On the first launch as Miru, its folder
/// (`%LOCALAPPDATA%\Nocturne`: library database, settings, artwork cache) is moved to the new
/// name and the artwork paths stored in the library are updated. If the move fails (say the old
/// version is still running), the old folder is used for now and the move is retried next time.
fn migrate_from_old_name(dirs: directories::ProjectDirs) -> directories::ProjectDirs {
    let Some(old) = directories::ProjectDirs::from("", "", "Nocturne") else {
        return dirs;
    };
    let root = |d: &directories::ProjectDirs| d.data_local_dir().parent().map(Path::to_path_buf);
    let (Some(old_root), Some(new_root)) = (root(&old), root(&dirs)) else {
        return dirs;
    };
    if new_root.exists() || !old_root.exists() {
        return dirs;
    }
    match std::fs::rename(&old_root, &new_root) {
        Ok(()) => {
            tracing::info!(
                from = %old_root.display(),
                to = %new_root.display(),
                "moved app data to the new name"
            );
            let db = dirs.data_local_dir().join("library.db");
            let rewritten = Db::open(&db).and_then(|db| {
                db.replace_path_prefix(&old_root.to_string_lossy(), &new_root.to_string_lossy())
            });
            if let Err(e) = rewritten {
                tracing::error!("updating artwork paths after the move: {e}");
            }
            dirs
        }
        Err(e) => {
            tracing::warn!(
                from = %old_root.display(),
                "cannot move app data yet, using the old folder: {e}"
            );
            old
        }
    }
}

impl AppPaths {
    /// Uses the platform data and cache folders, or `MIRU_DATA_DIR` when set (handy for
    /// development and tests).
    pub fn resolve() -> Result<Self> {
        if let Some(dir) = std::env::var_os("MIRU_DATA_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            return Ok(Self {
                db: dir.join("library.db"),
                artwork: dir.join("artwork"),
            });
        }
        let dirs = directories::ProjectDirs::from("", "", "Miru")
            .context("cannot determine a home directory")?;
        let dirs = migrate_from_old_name(dirs);
        let data = dirs.data_local_dir();
        std::fs::create_dir_all(data).with_context(|| format!("creating {}", data.display()))?;
        Ok(Self {
            db: data.join("library.db"),
            artwork: dirs.cache_dir().join("artwork"),
        })
    }
}

/// Scans every library root and returns the combined report.
pub fn scan_all(paths: &AppPaths) -> Result<ScanReport> {
    let mut db = Db::open(&paths.db)?;
    let mut total = ScanReport::default();
    for root in db.roots()? {
        let r = scan_root(&mut db, &root)?;
        total.added += r.added;
        total.updated += r.updated;
        total.unchanged += r.unchanged;
        total.removed += r.removed;
        total.errors += r.errors;
    }
    // Every root has now been parsed with the current rules.
    db.set_parser_version(miru_core::parse::PARSER_VERSION)?;
    Ok(total)
}

/// Fills in artwork and metadata for a batch of titles; returns how many were saved.
pub fn enrich_batch(paths: &AppPaths, tmdb: Option<&Tmdb>, batch: u32) -> Result<usize> {
    let db = Db::open(&paths.db)?;
    let cache = ArtworkCache::new(&paths.artwork)?;
    Ok(enrich_missing(&db, tmdb, &cache, batch)?)
}

/// Episode thumbnails are grabbed this far into the file, to skip cold opens and title cards.
const FRAME_POSITION: f64 = 0.2;
const FRAME_WIDTH: u32 = 640;

/// Fills in stills and details for a batch of one show's episodes; returns how many were saved.
pub fn enrich_show_episodes(
    paths: &AppPaths,
    tmdb: Option<&Tmdb>,
    show: &str,
    batch: u32,
) -> Result<usize> {
    let db = Db::open(&paths.db)?;
    let cache = ArtworkCache::new(&paths.artwork)?;
    let grab = |video: &std::path::Path, out: &std::path::Path| {
        miru_player::capture_frame(video, out, FRAME_POSITION, FRAME_WIDTH)
            .map_err(|e| e.to_string())
    };
    Ok(miru_metadata::enrich_episodes(
        &db, tmdb, &cache, show, batch, &grab,
    )?)
}

/// A TMDB entry offered in Fix match, with its poster thumbnail when one could be fetched.
#[derive(Clone)]
pub struct MatchCandidate {
    pub found: TmdbMatch,
    pub kind: ItemKind,
    pub thumb: Option<PathBuf>,
}

/// Results shown in Fix match.
const MATCH_CANDIDATES: usize = 12;

/// Searches TMDB for Fix match and fetches the candidates' poster thumbnails.
pub fn match_candidates(
    paths: &AppPaths,
    tmdb: &Tmdb,
    kind: ItemKind,
    query: &str,
) -> Result<Vec<MatchCandidate>> {
    let cache = ArtworkCache::new(&paths.artwork)?;
    let results = tmdb.search_query(kind, query)?;
    Ok(results
        .into_iter()
        .take(MATCH_CANDIDATES)
        .map(|found| {
            let thumb = found.poster_path.as_deref().and_then(|remote| {
                cache
                    .thumbnail(tmdb, remote)
                    .inspect_err(|e| tracing::warn!("thumbnail download failed: {e}"))
                    .ok()
            });
            MatchCandidate { found, kind, thumb }
        })
        .collect())
}

/// Saves the user's Fix match choice and fetches the title's new details and artwork.
pub fn apply_match(
    paths: &AppPaths,
    tmdb: &Tmdb,
    item: &LibraryItem,
    kind: ItemKind,
    id: i64,
) -> Result<()> {
    let db = Db::open(&paths.db)?;
    let cache = ArtworkCache::new(&paths.artwork)?;
    Ok(miru_metadata::apply_match(
        &db, tmdb, &cache, item, kind, id,
    )?)
}

/// What a person is known for on TMDB, with poster thumbnails (`None` where the download
/// failed).
pub fn known_for(
    paths: &AppPaths,
    tmdb: &Tmdb,
    person_id: i64,
    limit: usize,
) -> Result<Vec<(miru_metadata::KnownCredit, Option<PathBuf>)>> {
    let cache = ArtworkCache::new(&paths.artwork)?;
    Ok(tmdb
        .known_for(person_id, limit)?
        .into_iter()
        .map(|credit| {
            let thumb = credit.poster_path.as_deref().and_then(|remote| {
                cache
                    .thumbnail(tmdb, remote)
                    .inspect_err(|e| tracing::warn!("thumbnail download failed: {e}"))
                    .ok()
            });
            (credit, thumb)
        })
        .collect())
}

/// Downloads cast photos that aren't cached yet; returns `(person_id, file)` for every photo
/// available. Failures are logged and skipped (the pane shows initials instead).
pub fn cast_photos(
    paths: &AppPaths,
    tmdb: &Tmdb,
    people: &[(i64, String)],
) -> Result<Vec<(i64, PathBuf)>> {
    let cache = ArtworkCache::new(&paths.artwork)?;
    Ok(people
        .iter()
        .filter_map(|(id, remote)| {
            cache
                .profile(tmdb, *id, remote)
                .inspect_err(|e| tracing::warn!(person = id, "cast photo download failed: {e}"))
                .ok()
                .map(|path| (*id, path))
        })
        .collect())
}
