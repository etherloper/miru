//! Updates: find a newer release, download its installer and run it.
//!
//! Releases are read from a GitHub repository set when the app is built
//! (`MIRU_UPDATE_REPO=owner/name`). The app reads it without signing in, so that repository
//! must be public; the code itself can live in a private one. A release is a tag like `v0.2.0`
//! with the Windows installer (`*-setup.exe`) attached.
//!
//! All calls are blocking; run them off the UI thread.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Repository whose releases this build follows: `MIRU_UPDATE_REPO` when the build ran (a fork
/// sets its own), otherwise Miru's own repository, so every build can check for updates.
pub const UPDATE_REPO: Option<&str> = match option_env!("MIRU_UPDATE_REPO") {
    Some(repo) => Some(repo),
    None => Some("etherloper/miru"),
};
/// Automatic checks run at most this often.
pub const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// A newer version, ready to download.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    pub installer_url: String,
    pub installer_name: String,
    pub size: u64,
    /// SHA-256 of the installer when GitHub reports one, checked after downloading.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// Parses `1.2.3` or `v1.2.3` (anything after a `-` or `+` is ignored).
    pub fn parse(text: &str) -> Option<Self> {
        let core = text
            .trim()
            .trim_start_matches(['v', 'V'])
            .split(['-', '+'])
            .next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
        let version = Self(
            parts.next()??,
            parts.next().unwrap_or(Some(0))?,
            parts.next().unwrap_or(Some(0))?,
        );
        Some(version)
    }

    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>` on recent releases.
    #[serde(default)]
    digest: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("Miru/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The latest release of `repo` if it is newer than `current`.
pub fn check(repo: &str, current: Version) -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response: GitHubRelease = match agent()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(mut response) => response.body_mut().read_json()?,
        // No releases published yet.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(e).context("asking GitHub for the latest release"),
    };
    Ok(newer_release(response, current))
}

fn newer_release(release: GitHubRelease, current: Version) -> Option<Release> {
    if release.draft || release.prerelease {
        return None;
    }
    let version = Version::parse(&release.tag_name)?;
    if version <= current {
        return None;
    }
    let installer = release
        .assets
        .into_iter()
        .find(|a| a.name.to_lowercase().ends_with("-setup.exe"))?;
    Some(Release {
        version,
        notes: release.body.unwrap_or_default().trim().to_string(),
        installer_url: installer.browser_download_url,
        installer_name: installer.name,
        size: installer.size,
        sha256: installer
            .digest
            .and_then(|d| d.strip_prefix("sha256:").map(str::to_lowercase)),
    })
}

/// Downloads the installer into `dir`, reporting progress (0–1) as it goes, and checks its
/// SHA-256 when one is known.
pub fn download(release: &Release, dir: &Path, progress: impl Fn(f32)) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join(&release.installer_name);
    let partial = target.with_extension("part");
    let mut response = agent()
        .get(&release.installer_url)
        .call()
        .context("downloading the update")?;
    let mut reader = response.body_mut().with_config().limit(1 << 30).reader();
    let mut file = std::fs::File::create(&partial)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..n])?;
        hasher.update(&buffer[..n]);
        done += n as u64;
        if release.size > 0 {
            progress((done as f32 / release.size as f32).min(1.));
        }
    }
    drop(file);
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if let Some(expected) = &release.sha256
        && *expected != digest
    {
        let _ = std::fs::remove_file(&partial);
        bail!("the downloaded update is damaged (checksum mismatch); try again");
    }
    std::fs::rename(&partial, &target)?;
    Ok(target)
}

/// Runs the installer silently once the app has quit, then starts the new version. The caller
/// quits straight after this returns.
#[cfg(windows)]
pub fn install_and_restart(installer: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let app = std::env::current_exe().context("finding the running app")?;
    // PowerShell single-quoted strings: double any single quote in the paths.
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
    let script = format!(
        "Start-Sleep -Seconds 2; \
         Start-Process -Wait -FilePath {installer} -ArgumentList '/S'; \
         Start-Process -FilePath {app}",
        installer = quote(installer),
        app = quote(&app),
    );
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
        ])
        .arg(script)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("starting the installer")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn install_and_restart(_: &Path) -> Result<()> {
    bail!("installing updates is only supported on Windows for now")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            body: Some(" Fixes. ".into()),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|name| GitHubAsset {
                    name: (*name).into(),
                    browser_download_url: format!("https://example.invalid/{name}"),
                    size: 10,
                    digest: Some("sha256:ABCD".into()),
                })
                .collect(),
        }
    }

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("0.4"), Some(Version(0, 4, 0)));
        assert_eq!(Version::parse("2.0.0-beta.1"), Some(Version(2, 0, 0)));
        assert_eq!(Version::parse("latest"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
    }

    #[test]
    fn only_newer_releases_with_an_installer_count() {
        let current = Version(0, 1, 0);
        let found = newer_release(
            release("v0.2.0", &["notes.txt", "miru_0.2.0_x64-setup.exe"]),
            current,
        )
        .unwrap();
        assert_eq!(found.version, Version(0, 2, 0));
        assert_eq!(found.installer_name, "miru_0.2.0_x64-setup.exe");
        assert_eq!(found.sha256.as_deref(), Some("abcd"));
        assert_eq!(found.notes, "Fixes.");

        assert!(newer_release(release("v0.1.0", &["x-setup.exe"]), current).is_none());
        assert!(newer_release(release("v0.3.0", &["portable.zip"]), current).is_none());
        let mut pre = release("v0.3.0", &["x-setup.exe"]);
        pre.prerelease = true;
        assert!(newer_release(pre, current).is_none());
    }
}
