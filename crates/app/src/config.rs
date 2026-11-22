//! Secrets such as the TMDB key.
//!
//! Lookup order: a key entered in Settings; then the process environment, `.env` in the working
//! directory (handy when running from the project folder) and `.env` in the app's data folder.
//! No key is built into the app: people add their own in Settings → Metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::library_sync::AppPaths;

pub const TMDB_KEY: &str = "TMDB_API_KEY";

/// Where the TMDB key in use came from, for Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Settings,
    Environment,
}

pub fn tmdb_key(paths: &AppPaths, from_settings: Option<&str>) -> Option<(String, KeySource)> {
    if let Some(key) = from_settings.map(str::trim).filter(|k| !k.is_empty()) {
        return Some((key.to_string(), KeySource::Settings));
    }
    let mut candidates = vec![PathBuf::from(".env")];
    if let Some(data_dir) = paths.db.parent() {
        candidates.push(data_dir.join(".env"));
    }
    lookup(TMDB_KEY, &candidates).map(|key| (key, KeySource::Environment))
}

fn lookup(name: &str, env_files: &[PathBuf]) -> Option<String> {
    let non_empty = |v: String| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    if let Some(value) = std::env::var(name).ok().and_then(non_empty) {
        return Some(value);
    }
    env_files
        .iter()
        .find_map(|path| read_env_file(path).remove(name).and_then(non_empty))
}

fn read_env_file(path: &Path) -> HashMap<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(e) => {
            tracing::warn!("cannot read {}: {e}", path.display());
            HashMap::new()
        }
    }
}

/// Parses `KEY=value` lines. Supports comments, blank lines, `export KEY=...` and quoted values.
fn parse(text: &str) -> HashMap<String, String> {
    // Some Windows editors prepend a UTF-8 byte-order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let unquoted = ['"', '\'']
                .iter()
                .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
                .unwrap_or(value);
            Some((key.trim().to_string(), unquoted.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_dotenv_forms() {
        assert_eq!(parse("\u{feff}A=1")["A"], "1");
        let vars = parse(
            "# comment\n\nTMDB_API_KEY=abc123\nexport QUOTED=\"with spaces\"\nSINGLE='x=y'\n  SPACED = v  \nnot a pair\n",
        );
        assert_eq!(vars["TMDB_API_KEY"], "abc123");
        assert_eq!(vars["QUOTED"], "with spaces");
        assert_eq!(vars["SINGLE"], "x=y");
        assert_eq!(vars["SPACED"], "v");
        assert_eq!(vars.len(), 4);
    }

    #[test]
    fn falls_back_through_env_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.env");
        let second = dir.path().join("second.env");
        std::fs::write(&first, "OTHER=1\nMIRU_TEST_ONLY_KEY=\n").unwrap();
        std::fs::write(&second, "MIRU_TEST_ONLY_KEY=from-second\n").unwrap();
        let missing = dir.path().join("missing.env");
        let files = [missing, first, second];
        // Empty values are skipped, missing files ignored.
        assert_eq!(
            lookup("MIRU_TEST_ONLY_KEY", &files).as_deref(),
            Some("from-second")
        );
        assert_eq!(lookup("MIRU_TEST_ABSENT", &files), None);
    }
}
