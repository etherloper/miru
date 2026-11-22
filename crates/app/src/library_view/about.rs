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
    /// Downloading or starting the installer failed; the release is kept for a retry.
    DownloadFailed {
        release: Release,
        message: SharedString,
    },
}

/// `MIRU_PREVIEW_UPDATE=1`: pretend an update is available and simulate downloading it, to
/// check the update UI without a real release.
fn preview_mode() -> bool {
    std::env::var_os("MIRU_PREVIEW_UPDATE").is_some()
}

/// The pretend update offered in preview mode: the next patch version.
fn preview_release() -> UpdateState {
    let Version(major, minor, patch) = Version::current();
    UpdateState::Available(Release {
        version: Version(major, minor, patch + 1),
        notes: "Preview of the update notice.".into(),
        installer_url: String::new(),
        installer_name: "preview-setup.exe".into(),
        size: 0,
        sha256: None,
    })
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
        if preview_mode() {
            self.update = preview_release();
            return;
        }
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
        if preview_mode() {
            // Pretend to check, then offer the pretend update again.
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                this.update(cx, |view, cx| {
                    view.update = preview_release();
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
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

    pub(super) fn download_update(&mut self, cx: &mut Context<Self>) {
        let release = match &self.update {
            UpdateState::Available(release) | UpdateState::DownloadFailed { release, .. } => {
                release.clone()
            }
            _ => return,
        };
        if preview_mode() {
            self.simulate_download(release, cx);
            return;
        }
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
                        UpdateState::DownloadFailed {
                            release,
                            message: format!("Download failed: {e:#}").into(),
                        }
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn install_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready { installer, release } = &self.update else {
            return;
        };
        let started = if preview_mode() {
            Err(anyhow::anyhow!(
                "this is only a preview, so there is nothing to install"
            ))
        } else {
            updates::install_and_restart(installer)
        };
        match started {
            Ok(()) => cx.quit(),
            Err(e) => {
                self.update = UpdateState::DownloadFailed {
                    release: release.clone(),
                    message: format!("Couldn't start the installer: {e:#}").into(),
                };
                cx.notify();
            }
        }
    }

    /// Preview mode: fills the progress over a few seconds, then reports the update ready.
    fn simulate_download(&mut self, release: Release, cx: &mut Context<Self>) {
        self.update = UpdateState::Downloading {
            release: release.clone(),
            progress: 0.,
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            for step in 1..=40 {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let alive = this.update(cx, |view, cx| {
                    if let UpdateState::Downloading { progress, .. } = &mut view.update {
                        *progress = step as f32 / 40.;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
            this.update(cx, |view, cx| {
                view.update = UpdateState::Ready {
                    release,
                    installer: PathBuf::from("preview-setup.exe"),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
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
            UpdateState::Idle => (format!("You have version {current}."), None),
            UpdateState::Checking => ("Checking for updates…".into(), None),
            UpdateState::UpToDate => (format!("Version {current} is the latest."), None),
            UpdateState::Available(release) => (
                format!(
                    "Version {} is available (you have {current}).{}",
                    release.version,
                    notes_suffix(&release.notes)
                ),
                None,
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
            UpdateState::Failed(message) => (message.to_string(), None),
            UpdateState::DownloadFailed { message, .. } => (message.to_string(), None),
        };
        // The refresh button sits beside whatever the row offers, except mid-check or download.
        let control = div()
            .flex()
            .items_center()
            .gap_3()
            .children(control)
            .children(self.update_icon_button(cx))
            .into_any_element();
        let control = Some(control);
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

    /// The borderless icon beside the update status: refresh to check for a new version, or a
    /// download arrow once one is available (or its download failed). None while busy.
    fn update_icon_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (icon_path, tooltip): (&'static str, SharedString) = match &self.update {
            UpdateState::Idle | UpdateState::UpToDate | UpdateState::Failed(_) => {
                ("icons/restart.svg", "Check for updates".into())
            }
            UpdateState::Available(release) => (
                "icons/download.svg",
                format!("Download version {}", release.version).into(),
            ),
            UpdateState::DownloadFailed { .. } => ("icons/download.svg", "Try again".into()),
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. } => {
                return None;
            }
        };
        let download = icon_path == "icons/download.svg";
        Some(
            div()
                .id("update-icon")
                .tab_index(0)
                .focus_visible(theme::focus_ring)
                .size(px(34.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(theme::glass_hover()))
                .child(icon(icon_path, 17., theme::text()))
                .tooltip(theme::text_tooltip_owned(tooltip))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if download {
                        this.download_update(cx);
                    } else {
                        this.check_for_updates(cx);
                    }
                }))
                .into_any_element(),
        )
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

impl LibraryView {
    /// The sidebar's update button, which runs the whole update in place: "Update available"
    /// starts the download, the button fills as it downloads, then "Restart to update" installs.
    /// A failed download offers a retry. `None` when no update is waiting.
    pub(super) fn update_button(
        &self,
        collapsed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        const RADIUS: f32 = 8.;
        let (label, icon_path, progress, action): (
            String,
            &'static str,
            Option<f32>,
            Option<UpdateAction>,
        ) = match &self.update {
            UpdateState::Available(_) => (
                "Update available".into(),
                "icons/download.svg",
                None,
                Some(UpdateAction::Download),
            ),
            UpdateState::Downloading { progress, .. } => (
                format!("Downloading… {}%", (progress * 100.).round()),
                "icons/download.svg",
                Some(*progress),
                None,
            ),
            UpdateState::Ready { .. } => (
                "Restart to update".into(),
                "icons/restart.svg",
                None,
                Some(UpdateAction::Install),
            ),
            UpdateState::DownloadFailed { .. } => (
                "Update failed · Retry".into(),
                "icons/restart.svg",
                None,
                Some(UpdateAction::Download),
            ),
            _ => return None,
        };
        let tooltip: SharedString = match &self.update {
            UpdateState::DownloadFailed { message, .. } => message.clone(),
            _ => label.clone().into(),
        };
        // While downloading, a dimmed base with a white fill growing from the left; otherwise
        // solid white. The fill rounds its own corners (children aren't clipped to rounding).
        let progress = progress.map(|target| self.glide_fill(target, window));
        let fill = progress.map(|p| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(gpui::relative(p.clamp(0., 1.)))
                .bg(theme::text())
                .rounded_l(px(RADIUS))
                .when(p > 0.97, |d| d.rounded_r(px(RADIUS)))
        });
        let button = div()
            .id("nav-update")
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .relative()
            .mx_3()
            .mb_1()
            .h(px(NAV_ITEM_H))
            .px_3()
            .flex()
            .items_center()
            .when(collapsed, |d| d.justify_center())
            .gap_3()
            .rounded(px(RADIUS))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::bg())
            .shadow(theme::card_shadow(0.))
            .bg(if progress.is_some() {
                rgba(0xffffff73)
            } else {
                theme::text()
            })
            .children(fill)
            .child(
                div()
                    .relative()
                    .flex()
                    .child(icon(icon_path, 18., theme::bg())),
            )
            .when(!collapsed, |d| {
                d.child(div().relative().truncate().child(label))
            })
            .when(collapsed, |d| {
                d.tooltip(theme::text_tooltip_owned(tooltip.clone()))
            })
            .when(
                !collapsed && matches!(self.update, UpdateState::DownloadFailed { .. }),
                |d| d.tooltip(theme::text_tooltip_owned(tooltip.clone())),
            );
        let button = match action {
            Some(action) => button
                .cursor_pointer()
                .hover(|s| s.bg(rgba(0xffffffd9)))
                .on_click(cx.listener(move |this, _, _, cx| match action {
                    UpdateAction::Download => this.download_update(cx),
                    UpdateAction::Install => this.install_update(cx),
                })),
            None => button,
        };
        Some(button.into_any_element())
    }
}

impl LibraryView {
    /// Download progress as drawn: it eases toward the real value every frame, so the fill
    /// glides instead of jumping each time a chunk arrives.
    fn glide_fill(&self, target: f32, window: &mut Window) -> f32 {
        let (shown, at) = self.update_fill.get();
        let dt = at.elapsed().as_secs_f32().min(0.1);
        let next = if target < shown {
            target // a new download starting over
        } else {
            shown + (target - shown) * (1. - (-dt / FILL_EASE_SECS).exp())
        };
        self.update_fill.set((next, Instant::now()));
        if (target - next).abs() > 0.001 {
            window.request_animation_frame();
        }
        next
    }
}

/// Time constant of the download fill's easing (seconds).
const FILL_EASE_SECS: f32 = 0.15;

#[derive(Clone, Copy)]
enum UpdateAction {
    Download,
    Install,
}
