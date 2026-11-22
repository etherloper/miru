//! Settings: the TMDB key field, update checks and installs, and the About details.

use crate::config::KeySource;
use crate::updates::{self, Release, Version};

use super::*;

/// Where an update stands.
#[derive(Clone, PartialEq, Default)]
pub(super) enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading {
        release: Release,
        progress: f32,
    },
    /// Downloaded and verified; installing restarts the app.
    Ready {
        release: Release,
        installer: PathBuf,
    },
    Failed(SharedString),
}

impl UpdateState {
    pub fn is_available(&self) -> bool {
        matches!(
            self,
            Self::Available(_) | Self::Downloading { .. } | Self::Ready { .. }
        )
    }
}

impl LibraryView {
    // ---- TMDB key -----------------------------------------------------------------------

    /// Rebuilds the TMDB client after the key changed, then syncs so titles waiting for
    /// metadata are looked up with it.
    fn apply_tmdb_key(&mut self, cx: &mut Context<Self>) {
        let key = crate::config::tmdb_key(&self.paths, self.settings.tmdb_key.as_deref());
        self.tmdb_source = key.as_ref().map(|(_, source)| *source);
        self.tmdb = key.map(|(key, _)| Tmdb::new(&key));
        self.start_sync(cx);
        cx.notify();
    }

    fn start_key_edit(&mut self, cx: &mut Context<Self>) {
        self.key_draft = Some(String::new());
        self.key_focus_pending = true;
        cx.notify();
    }

    fn save_key(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.key_draft.take() else {
            return;
        };
        let key = draft.trim().to_string();
        if !key.is_empty() {
            self.update_settings(|s| s.tmdb_key = Some(key), cx);
            self.apply_tmdb_key(cx);
        }
        cx.notify();
    }

    fn remove_key(&mut self, cx: &mut Context<Self>) {
        self.update_settings(|s| s.tmdb_key = None, cx);
        self.apply_tmdb_key(cx);
    }

    pub(super) fn on_key_field_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = &mut self.key_draft else {
            return;
        };
        let k = &event.keystroke;
        let command = k.modifiers.control || k.modifiers.platform;
        match k.key.as_str() {
            "enter" => {
                self.save_key(cx);
                window.focus(&self.focus, cx);
            }
            "escape" => {
                self.key_draft = None;
                window.focus(&self.focus, cx);
            }
            "backspace" if command => draft.clear(),
            "backspace" => {
                draft.pop();
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    draft.extend(text.chars().filter(|c| !c.is_whitespace()));
                }
            }
            _ if command || k.modifiers.alt => return,
            _ => match &k.key_char {
                Some(typed) if !typed.chars().any(char::is_whitespace) => draft.push_str(typed),
                _ => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Rows for the Metadata section: where the TMDB key comes from, and setting your own.
    pub(super) fn metadata_rows(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let description = match self.tmdb_source {
            Some(KeySource::Settings) => "Using the key you added.",
            Some(KeySource::Environment) => "Using the key from your .env file.",
            None => {
                "Add a TMDB key to get posters, artwork and details. Without one, artwork comes \
                 from your folders only. Keys are free at themoviedb.org."
            }
        };
        let control: AnyElement =
            match &self.key_draft {
                Some(draft) => {
                    let focused = self.key_focus.is_focused(window);
                    // Shown masked: it is a secret, even if only an API key.
                    let masked: String = draft.chars().map(|_| '•').collect();
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("tmdb-key-field")
                                .track_focus(&self.key_focus)
                                .on_key_down(cx.listener(Self::on_key_field_key))
                                .w(px(240.))
                                .h(px(36.))
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded(px(8.))
                                .bg(rgba(0xffffff0d))
                                .border_1()
                                .border_color(if focused {
                                    rgba(0xffffff4d)
                                } else {
                                    theme::glass_border()
                                })
                                .text_sm()
                                .overflow_hidden()
                                .cursor_text()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    window.focus(&this.key_focus, cx)
                                }))
                                .child(if draft.is_empty() {
                                    div()
                                        .text_color(theme::text_tertiary())
                                        .child("Paste your key")
                                        .into_any_element()
                                } else {
                                    div().truncate().child(masked).into_any_element()
                                }),
                        )
                        .child(small_button(
                            "tmdb-key-save",
                            "Save",
                            false,
                            cx.listener(|this, _, window, cx| {
                                this.save_key(cx);
                                window.focus(&this.focus, cx);
                            }),
                        ))
                        .child(small_button(
                            "tmdb-key-cancel",
                            "Cancel",
                            false,
                            cx.listener(|this, _, _, cx| {
                                this.key_draft = None;
                                cx.notify();
                            }),
                        ))
                        .into_any_element()
                }
                None => div()
                    .flex()
                    .gap_2()
                    .child(small_button(
                        "tmdb-key-edit",
                        if self.settings.tmdb_key.is_some() {
                            "Change key"
                        } else {
                            "Add key"
                        },
                        false,
                        cx.listener(|this, _, _, cx| this.start_key_edit(cx)),
                    ))
                    .when(self.settings.tmdb_key.is_some(), |d| {
                        d.child(small_button(
                            "tmdb-key-remove",
                            "Remove",
                            false,
                            cx.listener(|this, _, _, cx| this.remove_key(cx)),
                        ))
                    })
                    .into_any_element(),
            };
        vec![setting_row("TMDB key", Some(description), Some(control)).into_any_element()]
    }

    // ---- updates ------------------------------------------------------------------------

    /// Checks on launch when automatic checks are on and the last one was a day ago.
    pub(super) fn auto_check_for_updates(&mut self, cx: &mut Context<Self>) {
        let due = unix_now() - self.settings.update_checked_at >= updates::CHECK_INTERVAL_SECS;
        if self.settings.auto_update && due && updates::UPDATE_REPO.is_some() {
            self.check_for_updates(cx);
        }
    }

    pub(super) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = updates::UPDATE_REPO else {
            return;
        };
        if matches!(
            self.update,
            UpdateState::Checking | UpdateState::Downloading { .. }
        ) {
            return;
        }
        self.update = UpdateState::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { updates::check(repo, Version::current()) })
                .await;
            this.update(cx, |view, cx| {
                view.update = match found {
                    Ok(Some(release)) => {
                        tracing::info!(version = %release.version, "update available");
                        UpdateState::Available(release)
                    }
                    Ok(None) => UpdateState::UpToDate,
                    Err(e) => {
                        tracing::warn!("update check failed: {e:#}");
                        UpdateState::Failed(format!("Couldn't check for updates: {e:#}").into())
                    }
                };
                view.update_settings(|s| s.update_checked_at = unix_now(), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn download_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = &self.update else {
            return;
        };
        let release = release.clone();
        self.update = UpdateState::Downloading {
            release: release.clone(),
            progress: 0.,
        };
        cx.notify();
        let dir = std::env::temp_dir().join("miru-update");
        let (progress_tx, progress_rx) = async_channel::unbounded::<f32>();
        // Progress arrives from the download thread; show it as it comes.
        cx.spawn(async move |this, cx| {
            while let Ok(progress) = progress_rx.recv().await {
                let alive = this.update(cx, |view, cx| {
                    if let UpdateState::Downloading { progress: p, .. } = &mut view.update {
                        *p = progress;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let job_release = release.clone();
            let downloaded = cx
                .background_executor()
                .spawn(async move {
                    updates::download(&job_release, &dir, |p| {
                        let _ = progress_tx.try_send(p);
                    })
                })
                .await;
            this.update(cx, |view, cx| {
                view.update = match downloaded {
                    Ok(installer) => UpdateState::Ready { release, installer },
                    Err(e) => {
                        tracing::warn!("update download failed: {e:#}");
                        UpdateState::Failed(format!("Download failed: {e:#}").into())
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn install_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready { installer, .. } = &self.update else {
            return;
        };
        match updates::install_and_restart(installer) {
            Ok(()) => cx.quit(),
            Err(e) => {
                self.update =
                    UpdateState::Failed(format!("Couldn't start the installer: {e:#}").into());
                cx.notify();
            }
        }
    }

    /// Update rows for About: status with the next action, and automatic checks.
    pub(super) fn update_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if updates::UPDATE_REPO.is_none() {
            return vec![
                setting_row(
                    "Updates",
                    Some("This build isn't set up to check for updates."),
                    None,
                )
                .into_any_element(),
            ];
        }
        let current = Version::current();
        let (description, control): (String, Option<AnyElement>) = match &self.update {
            UpdateState::Idle => (
                format!("You have version {current}."),
                Some(self.check_button(cx)),
            ),
            UpdateState::Checking => ("Checking for updates…".into(), None),
            UpdateState::UpToDate => (
                format!("Version {current} is the latest."),
                Some(self.check_button(cx)),
            ),
            UpdateState::Available(release) => (
                format!(
                    "Version {} is available (you have {current}).{}",
                    release.version,
                    notes_suffix(&release.notes)
                ),
                Some(
                    primary_button(
                        "update-download",
                        None,
                        "Download",
                        cx.listener(|this, _, _, cx| this.download_update(cx)),
                    )
                    .into_any_element(),
                ),
            ),
            UpdateState::Downloading { release, progress } => (
                format!(
                    "Downloading version {}… {}%",
                    release.version,
                    (progress * 100.).round()
                ),
                Some(
                    div()
                        .w(px(140.))
                        .child(theme::progress_bar(*progress, 4.))
                        .into_any_element(),
                ),
            ),
            UpdateState::Ready { release, .. } => (
                format!(
                    "Version {} is ready. Miru will close, update and reopen.",
                    release.version
                ),
                Some(
                    primary_button(
                        "update-install",
                        None,
                        "Install and restart",
                        cx.listener(|this, _, _, cx| this.install_update(cx)),
                    )
                    .into_any_element(),
                ),
            ),
            UpdateState::Failed(message) => (message.to_string(), Some(self.check_button(cx))),
        };
        let auto = toggle(
            "update-auto",
            self.settings.auto_update,
            cx.listener(|this, _, _, cx| this.update_settings(|s| s.auto_update ^= true, cx)),
        );
        vec![
            setting_row("Updates", Some(&description), control).into_any_element(),
            setting_row(
                "Check automatically",
                Some("Look for a new version when Miru starts, at most once a day."),
                Some(auto.into_any_element()),
            )
            .into_any_element(),
        ]
    }

    fn check_button(&self, cx: &mut Context<Self>) -> AnyElement {
        small_button(
            "update-check",
            "Check now",
            false,
            cx.listener(|this, _, _, cx| this.check_for_updates(cx)),
        )
        .into_any_element()
    }
}

/// The first line of release notes, appended to the update status.
fn notes_suffix(notes: &str) -> String {
    notes
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|line| format!(" {line}"))
        .unwrap_or_default()
}

/// TMDB requires apps using its API to say so.
pub(super) fn attribution_row() -> AnyElement {
    setting_row(
        "Metadata",
        Some(
            "Posters, artwork and details come from TMDB. This product uses the TMDB API but is \
             not endorsed or certified by TMDB.",
        ),
        None,
    )
    .into_any_element()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
