//! Settings page and the form controls it is built from.

use super::*;
use miru_core::tracks::{FileTrackPrefs, TrackPrefs};
use miru_player::TrackKind;

/// Languages offered for preferred audio and subtitles: (label, ISO 639-1 code).
const LANGUAGES: [(&str, Option<&str>); 7] = [
    ("Default", None),
    ("English", Some("en")),
    ("Spanish", Some("es")),
    ("French", Some("fr")),
    ("German", Some("de")),
    ("Japanese", Some("ja")),
    ("Korean", Some("ko")),
];

impl LibraryView {
    /// Artwork for the system media controls: the movie's or show's poster.
    pub fn artwork_for(&self, file_id: i64) -> Option<String> {
        let poster = |item: &LibraryItem| item.metadata.as_ref()?.poster_file.clone();
        if let Some(movie) = self.movies.iter().find(|m| m.file_id == file_id) {
            return poster(movie);
        }
        let show = self.show_of(file_id)?;
        self.shows
            .iter()
            .find(|s| s.key.title.eq_ignore_ascii_case(&show))
            .and_then(poster)
    }

    /// The show a library file belongs to, if it is an episode.
    fn show_of(&self, file_id: i64) -> Option<String> {
        let path = self.db.file_path(file_id).ok()??;
        match self.db.file_by_path(&path).ok()??.media {
            ParsedMedia::Episode { show, .. } => Some(show),
            ParsedMedia::Movie { .. } => None,
        }
    }

    /// Tracks last picked for the show `file_id` belongs to (nothing for movies), and the
    /// exact tracks last used for the file itself.
    pub fn track_prefs(&self, file_id: i64) -> (TrackPrefs, FileTrackPrefs) {
        let show = self
            .show_of(file_id)
            .and_then(|show| {
                self.db
                    .show_track_prefs(&show)
                    .inspect_err(|e| tracing::error!("reading track choices: {e}"))
                    .ok()
            })
            .unwrap_or_default();
        let file = self
            .db
            .file_track_prefs(file_id)
            .inspect_err(|e| tracing::error!("reading track choices: {e}"))
            .unwrap_or_default();
        (show, file)
    }

    /// Remembers a track picked while watching: for this file (to resume with it) and, for
    /// an episode, for the show's other episodes.
    pub fn remember_track(
        &mut self,
        file_id: i64,
        kind: TrackKind,
        lang: Option<&str>,
        title: Option<&str>,
        id: Option<i64>,
    ) {
        let saved = match (kind, id) {
            (TrackKind::Audio, Some(id)) => self.db.set_file_audio(file_id, id),
            (TrackKind::Audio, None) => Ok(()),
            (TrackKind::Subtitle, id) => self.db.set_file_subtitles(file_id, id),
        };
        if let Err(e) = saved {
            tracing::error!("saving track choice: {e}");
        }
        let Some(show) = self.show_of(file_id) else {
            return;
        };
        let saved = match (kind, id) {
            (TrackKind::Audio, Some(id)) => self.db.set_show_audio(&show, lang, title, id),
            (TrackKind::Audio, None) => Ok(()),
            (TrackKind::Subtitle, id) => self.db.set_show_subtitles(&show, lang, title, id),
        };
        if let Err(e) = saved {
            tracing::error!("saving track choice: {e}");
        }
    }

    /// Request for the episode after `file_id`, resuming it if it was started.
    pub fn next_episode_request(&self, file_id: i64) -> Option<PlaybackRequest> {
        let next = self.db.next_episode(file_id).ok().flatten()?;
        let show = match &next.media {
            ParsedMedia::Episode { show, .. } => show.clone(),
            ParsedMedia::Movie { .. } => return None,
        };
        let title = self
            .shows
            .iter()
            .find(|s| s.key.title.eq_ignore_ascii_case(&show))
            .map_or(show, |s| s.display_title().to_string());
        let progress = self.db.progress(next.id).ok().flatten().unwrap_or_default();
        Some(episode_request(&title, &next, &progress))
    }

    /// Saves the player's volume so the next video starts at the same level.
    pub fn remember_volume(&mut self, volume: u32) {
        if self.settings.volume == volume {
            return;
        }
        self.settings.volume = volume;
        if let Err(e) = self.db.save_settings(&self.settings) {
            tracing::warn!("cannot save volume: {e}");
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub(super) fn update_settings(
        &mut self,
        change: impl FnOnce(&mut Settings),
        cx: &mut Context<Self>,
    ) {
        change(&mut self.settings);
        if let Err(e) = self.db.save_settings(&self.settings) {
            self.show_error(format!("Could not save settings: {e}"), cx);
        }
        cx.notify();
    }

    pub(super) fn remove_root(&mut self, root_id: i64, cx: &mut Context<Self>) {
        self.confirm_remove = None;
        if let Err(e) = self.db.remove_root(root_id) {
            self.show_error(format!("Could not remove folder: {e}"), cx);
        }
        self.reload();
        cx.notify();
    }

    pub(super) fn render_settings(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = &self.settings;
        let busy = matches!(
            self.sync,
            SyncStatus::Scanning | SyncStatus::FetchingMetadata
        );

        // Library folders.
        let mut folders: Vec<AnyElement> = self
            .roots
            .iter()
            .map(|stats| self.folder_row(stats, cx))
            .collect();
        if folders.is_empty() {
            folders.push(
                setting_row(
                    "No folders yet",
                    Some("Add a folder of movies or TV shows to build your library."),
                    None,
                )
                .into_any_element(),
            );
        }
        let library_actions = div()
            .flex()
            .items_center()
            .gap_3()
            .px_5()
            .py_4()
            .child(primary_button(
                "settings-add-folder",
                Some("icons/plus.svg"),
                "Add Folder",
                cx.listener(Self::add_folders),
            ))
            .child(
                glass_button(
                    "settings-rescan",
                    Some("icons/restart.svg"),
                    if busy { "Scanning…" } else { "Scan Now" },
                    cx.listener(|this, _, _, cx| this.start_sync(cx)),
                )
                .when(busy, |d| d.opacity(0.6)),
            );

        // Playback.
        let seek = segmented(
            "seek-step",
            SEEK_STEPS
                .iter()
                .map(|&step| (format!("{step}s"), step == s.seek_step))
                .collect(),
            cx.listener(|this, ix: &usize, _, cx| {
                let step = SEEK_STEPS[*ix];
                this.update_settings(|s| s.seek_step = step, cx);
            }),
        );
        let audio_output = segmented(
            "audio-output",
            AudioOutput::ALL
                .iter()
                .map(|o| (o.label().to_string(), *o == s.audio_output))
                .collect(),
            cx.listener(|this, ix: &usize, _, cx| {
                let output = AudioOutput::ALL[*ix];
                this.update_settings(|s| s.audio_output = output, cx);
            }),
        );
        let subtitle_size = segmented(
            "subtitle-size",
            SubtitleSize::ALL
                .iter()
                .map(|z| (z.label().to_string(), *z == s.subtitle_size))
                .collect(),
            cx.listener(|this, ix: &usize, _, cx| {
                let size = SubtitleSize::ALL[*ix];
                this.update_settings(|s| s.subtitle_size = size, cx);
            }),
        );
        let subtitles = toggle(
            "subtitles-default",
            s.subtitles_by_default,
            cx.listener(|this, _, _, cx| {
                this.update_settings(|s| s.subtitles_by_default = !s.subtitles_by_default, cx);
            }),
        );
        let audio_lang = language_picker(
            "audio-lang",
            s.audio_language.as_deref(),
            cx.listener(|this, ix: &usize, _, cx| {
                let code = LANGUAGES[*ix].1.map(str::to_string);
                this.update_settings(|s| s.audio_language = code, cx);
            }),
        );
        let subtitle_lang = language_picker(
            "subtitle-lang",
            s.subtitle_language.as_deref(),
            cx.listener(|this, ix: &usize, _, cx| {
                let code = LANGUAGES[*ix].1.map(str::to_string);
                this.update_settings(|s| s.subtitle_language = code, cx);
            }),
        );

        // Home.
        let autoplay = toggle(
            "autoplay-next",
            s.autoplay_next,
            cx.listener(|this, _, _, cx| {
                this.update_settings(|s| s.autoplay_next = !s.autoplay_next, cx);
            }),
        );
        let rotate = toggle(
            "rotate-featured",
            s.rotate_featured,
            cx.listener(|this, _, _, cx| {
                this.update_settings(|s| s.rotate_featured = !s.rotate_featured, cx);
            }),
        );

        let whole_library = toggle(
            "feature-whole-library",
            s.feature_whole_library,
            cx.listener(|this, _, _, cx| {
                this.update_settings(|s| s.feature_whole_library = !s.feature_whole_library, cx);
                this.hero_avoid = None;
                this.refresh_featured();
                this.redraw_hero(cx);
            }),
        );

        let playback_rows = vec![
            setting_row(
                "Skip interval",
                Some("How far the skip buttons and arrow keys jump."),
                Some(seek.into_any_element()),
            )
            .into_any_element(),
            setting_row(
                "Subtitles",
                Some("Turn subtitles on automatically when a file has them."),
                Some(subtitles.into_any_element()),
            )
            .into_any_element(),
            setting_row(
                "Subtitle size",
                Some("Size of text subtitles. Takes effect the next time a video starts."),
                Some(subtitle_size.into_any_element()),
            )
            .into_any_element(),
            setting_row(
                "Autoplay next episode",
                Some("Start the next episode after a short countdown when one finishes."),
                Some(autoplay.into_any_element()),
            )
            .into_any_element(),
            stacked_row(
                "Audio output",
                "Stereo keeps dialogue clear on headphones and TV speakers. Source and 7.1 are for \
                 surround speaker systems.",
                // Wrapped so the pill keeps its natural width instead of stretching.
                div().flex().child(audio_output).into_any_element(),
            ),
            stacked_row(
                "Preferred audio language",
                "Picked automatically when a file has several audio tracks.",
                audio_lang.into_any_element(),
            ),
            stacked_row(
                "Preferred subtitle language",
                "Picked automatically when subtitles are on.",
                subtitle_lang.into_any_element(),
            ),
        ];
        let home_rows = vec![
            setting_row(
                "Rotate featured titles",
                Some("Cycle the large banner on Home every few seconds."),
                Some(rotate.into_any_element()),
            )
            .into_any_element(),
            setting_row(
                "Feature the whole library",
                Some("Pick featured titles from everything in your library, not just what was added recently."),
                Some(whole_library.into_any_element()),
            )
            .into_any_element(),
        ];
        let sidebar_rows = self.sidebar_order_rows(cx);
        let history_rows = self.history_rows(cx);
        let metadata_rows = self.metadata_rows(window, cx);
        let about_rows = self.about_rows(cx);
        folders.push(library_actions.into_any_element());

        div()
            .id("settings-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.detail_scroll)
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .pt(px(NAV_HEIGHT + 32.))
                    .pb_16()
                    .px(px(PAGE_PADDING))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(820.))
                            .flex()
                            .flex_col()
                            .gap_8()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .text_size(px(40.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Settings"),
                            )
                            .child(section(
                                "Library",
                                "Folders Miru scans for movies and TV shows. Files are never modified.",
                                folders,
                            ))
                            .child(section(
                                "Playback",
                                "Applies the next time you start playing something.",
                                playback_rows,
                            ))
                            .child(section("Home", "", home_rows))
                            .child(section(
                                "Sidebar",
                                "Drag to choose the order of the sidebar. Settings always stays at the bottom.",
                                sidebar_rows,
                            ))
                            .child(section(
                                "Metadata",
                                "Where posters, artwork and details come from.",
                                metadata_rows,
                            ))
                            .child(section(
                                "Watch History",
                                "Removing an item clears its resume point and watched mark. Files are not deleted.",
                                history_rows,
                            ))
                            .child(section(
                                "Keyboard shortcuts",
                                "Alt+← and the mouse back button work anywhere in the library.",
                                shortcut_rows(),
                            ))
                            .child(section("About", "", about_rows)),
                    ),
            )
    }

    pub(super) fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.confirm_clear_history = false;
        if let Err(e) = self.db.clear_history() {
            self.show_error(format!("Could not clear watch history: {e}"), cx);
        }
        self.refresh(cx);
    }

    /// Watch History, one row per title: a show's episodes collapse into one row showing the
    /// latest. The most recent few show at first; the rest are a click away.
    pub(super) fn history_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::new();
        if self.history.is_empty() {
            rows.push(
                setting_row(
                    "Nothing watched yet",
                    Some("Movies and episodes you play appear here."),
                    None,
                )
                .into_any_element(),
            );
            return rows;
        }

        // Group by title, keeping the order of each title's most recent play.
        struct Group<'a> {
            item: Option<LibraryItem>,
            title: String,
            latest: &'a (MediaFile, Progress, i64),
            file_ids: Vec<i64>,
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut index: HashMap<(bool, String, Option<u16>), usize> = HashMap::new();
        for entry in &self.history {
            let (file, _, _) = entry;
            let key = match &file.media {
                ParsedMedia::Movie { title, year } => (false, title.to_lowercase(), *year),
                ParsedMedia::Episode { show, .. } => (true, show.to_lowercase(), None),
            };
            if let Some(&ix) = index.get(&key) {
                groups[ix].file_ids.push(file.id);
                continue;
            }
            let item = match &file.media {
                ParsedMedia::Movie { title, year } => self
                    .movies
                    .iter()
                    .find(|m| m.key.title.eq_ignore_ascii_case(title) && m.key.year == *year),
                ParsedMedia::Episode { show, .. } => self
                    .shows
                    .iter()
                    .find(|s| s.key.title.eq_ignore_ascii_case(show)),
            }
            .cloned();
            let title = match (&item, &file.media) {
                (Some(item), _) => item.display_title().to_string(),
                (None, ParsedMedia::Movie { title, .. }) => title.clone(),
                (None, ParsedMedia::Episode { show, .. }) => show.clone(),
            };
            index.insert(key, groups.len());
            groups.push(Group {
                item,
                title,
                latest: entry,
                file_ids: vec![file.id],
            });
        }

        let total = groups.len();
        let shown = if self.history_expanded {
            total
        } else {
            total.min(HISTORY_PREVIEW)
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for (ix, group) in groups.into_iter().take(shown).enumerate() {
            let (file, progress, at) = group.latest;
            let mut details: Vec<String> = Vec::new();
            if let ParsedMedia::Episode { .. } = file.media {
                details.push(short_episode(&file.media));
            }
            details.push(if progress.watched {
                "Watched".to_string()
            } else {
                format!(
                    "{}% · {}",
                    (progress.fraction() * 100.).round() as u32,
                    remaining(progress)
                )
            });
            details.push(played_ago(now_ms - at));
            let episodes = group.file_ids.len();
            if matches!(file.media, ParsedMedia::Episode { .. }) && episodes > 1 {
                details.push(format!("{episodes} episodes"));
            }
            let meta = group.item.as_ref().and_then(|i| i.metadata.as_ref());
            let thumb = div()
                .relative()
                .w(px(36.))
                .h(px(54.))
                .flex_shrink_0()
                .rounded(px(5.))
                .overflow_hidden()
                .bg(theme::placeholder(
                    &group.title,
                    meta.and_then(|m| m.accent_rgb),
                ))
                .child(art_layer(
                    meta.and_then(|m| m.poster_file.clone()),
                    5.,
                    ART_THUMB,
                ));
            let ids = group.file_ids.clone();
            let key = group.item.as_ref().map(|i| i.key.clone());
            rows.push(
                div()
                    .id(("history-row", ix))
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_5()
                    .py_2p5()
                    .border_b_1()
                    .border_color(theme::glass_border())
                    .when(key.is_some(), |d| {
                        d.cursor_pointer().hover(|s| s.bg(rgba(0xffffff08)))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(key) = &key {
                            this.open_key(key, cx);
                        }
                    }))
                    .child(thumb)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(group.title.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::text_secondary())
                                    .truncate()
                                    .child(details.join("  ·  ")),
                            ),
                    )
                    .child(
                        theme::tinted_icon_button(
                            ("history-remove", ix),
                            "icons/trash.svg",
                            34.,
                            true,
                            theme::danger(),
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_titles_from_history(&ids, cx);
                            }),
                        )
                        .tooltip(theme::text_tooltip("Remove from history")),
                    )
                    .into_any_element(),
            );
        }
        if total > HISTORY_PREVIEW {
            let expanded = self.history_expanded;
            rows.push(
                div()
                    .id("history-more")
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme::glass_border())
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text_secondary())
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::text()))
                    .child(if expanded {
                        "Show fewer".to_string()
                    } else {
                        format!("Show all {total} titles")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.history_expanded ^= true;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let footer: AnyElement = if self.confirm_clear_history {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .mr_1()
                        .child("Clear all progress and watched marks?"),
                )
                .child(small_button(
                    "history-clear-cancel",
                    "Cancel",
                    false,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_clear_history = false;
                        cx.notify();
                    }),
                ))
                .child(small_button(
                    "history-clear-confirm",
                    "Clear History",
                    true,
                    cx.listener(|this, _, _, cx| this.clear_history(cx)),
                ))
                .into_any_element()
        } else {
            small_button(
                "history-clear",
                "Clear Watch History",
                false,
                cx.listener(|this, _, _, cx| {
                    this.confirm_clear_history = true;
                    cx.notify();
                }),
            )
            .into_any_element()
        };
        rows.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .px_5()
                .py_4()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::text_tertiary())
                        .child(if total == 1 {
                            "1 title".to_string()
                        } else {
                            format!("{total} titles")
                        }),
                )
                .child(footer)
                .into_any_element(),
        );
        rows
    }

    fn remove_titles_from_history(&mut self, file_ids: &[i64], cx: &mut Context<Self>) {
        for id in file_ids {
            if let Err(e) = self.db.remove_from_history(*id) {
                self.show_error(format!("Could not update watch history: {e}"), cx);
                break;
            }
        }
        self.refresh(cx);
    }

    pub(super) fn folder_row(&self, stats: &RootStats, cx: &mut Context<Self>) -> AnyElement {
        let id = stats.root.id;
        let confirming = self.confirm_remove == Some(id);
        let plural = |n: u32, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let counts = match (stats.movies, stats.episodes) {
            (0, 0) => "No videos found yet".to_string(),
            (m, 0) => plural(m, "movie"),
            (0, e) => plural(e, "episode"),
            (m, e) => format!("{} · {}", plural(m, "movie"), plural(e, "episode")),
        };
        let action: AnyElement = if confirming {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .mr_1()
                        .child("Remove from library?"),
                )
                .child(small_button(
                    ("cancel-remove", id as usize),
                    "Cancel",
                    false,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_remove = None;
                        cx.notify();
                    }),
                ))
                .child(small_button(
                    ("confirm-remove", id as usize),
                    "Remove",
                    true,
                    cx.listener(move |this, _, _, cx| this.remove_root(id, cx)),
                ))
                .into_any_element()
        } else {
            small_button(
                ("remove", id as usize),
                "Remove",
                false,
                cx.listener(move |this, _, _, cx| {
                    this.confirm_remove = Some(id);
                    cx.notify();
                }),
            )
            .into_any_element()
        };

        div()
            .flex()
            .items_center()
            .gap_4()
            .px_5()
            .py_4()
            .border_b_1()
            .border_color(theme::glass_border())
            .child(
                div()
                    .size(px(40.))
                    .flex_shrink_0()
                    .rounded(px(10.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::glass())
                    .child(icon("icons/folder.svg", 20., theme::text())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(stats.root.path.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::text_secondary())
                            .child(counts),
                    ),
            )
            .child(action)
            .into_any_element()
    }
}

/// A titled group of settings rows on a frosted card.
pub(super) fn section(
    title: &'static str,
    description: &'static str,
    rows: Vec<AnyElement>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .font_family(theme::FONT_DISPLAY)
                        .text_size(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .when(!description.is_empty(), |d| {
                    d.child(
                        div()
                            .text_sm()
                            .text_color(theme::text_secondary())
                            .child(description),
                    )
                }),
        )
        .child(
            div()
                .rounded(px(16.))
                .overflow_hidden()
                // Outlined on the black page rather than filled grey: on OLED panels a
                // near-black fill looks blotchy, while black with a hairline stays clean.
                .border_1()
                .border_color(rgba(0xffffff24))
                .flex()
                .flex_col()
                .children(rows),
        )
}

/// Label and description on the left, control on the right.
pub(super) fn setting_row(
    label: &str,
    description: Option<&str>,
    control: Option<AnyElement>,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_6()
        .px_5()
        .py_4()
        .border_b_1()
        .border_color(theme::glass_border())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_string()),
                )
                .children(description.map(|d| {
                    div()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .child(d.to_string())
                })),
        )
        .children(control)
}

/// Label and description with a wide control underneath.
pub(super) fn stacked_row(label: &str, description: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .px_5()
        .py_4()
        .border_b_1()
        .border_color(theme::glass_border())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::text_secondary())
                        .child(description.to_string()),
                ),
        )
        .child(control)
        .into_any_element()
}

type IndexHandler = dyn Fn(&usize, &mut Window, &mut gpui::App);

/// Pill-shaped segmented control; `on_select` receives the chosen index.
pub(super) fn segmented(
    id: &'static str,
    options: Vec<(String, bool)>,
    on_select: impl Fn(&usize, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let on_select: Rc<IndexHandler> = Rc::new(on_select);
    div()
        .flex()
        .flex_wrap()
        .flex_shrink_0()
        .gap_1()
        .p_1()
        .rounded(px(20.))
        .bg(theme::glass())
        .border_1()
        .border_color(theme::glass_border())
        .children(
            options
                .into_iter()
                .enumerate()
                .map(move |(ix, (label, selected))| {
                    let on_select = on_select.clone();
                    div()
                        .id((id, ix))
                        .tab_index(0)
                        .focus_visible(theme::focus_ring)
                        .h(px(30.))
                        .px_4()
                        .flex()
                        .items_center()
                        .rounded_full()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme::text()).text_color(theme::bg()))
                        .when(!selected, |d| {
                            d.text_color(theme::text_secondary())
                                .hover(|s| s.text_color(theme::text()))
                        })
                        .child(label)
                        .on_click(move |_, window, cx| on_select(&ix, window, cx))
                }),
        )
}

pub(super) fn language_picker(
    id: &'static str,
    current: Option<&str>,
    on_select: impl Fn(&usize, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let options = LANGUAGES
        .iter()
        .map(|(label, code)| (label.to_string(), *code == current))
        .collect();
    div().flex().child(segmented(id, options, on_select))
}

/// On/off switch.
pub(super) fn toggle(
    id: &'static str,
    on: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(theme::focus_ring)
        .w(px(46.))
        .h(px(28.))
        .flex_shrink_0()
        .p(px(3.))
        .rounded_full()
        .flex()
        .items_center()
        .cursor_pointer()
        .when(on, |d| d.bg(theme::accent()).justify_end())
        .when(!on, |d| d.bg(rgba(0xffffff29)).justify_start())
        .child(
            div()
                .size(px(22.))
                .rounded_full()
                .bg(theme::text())
                .shadow(theme::card_shadow(0.)),
        )
        .on_click(on_click)
}

pub(super) fn small_button(
    id: impl Into<ElementId>,
    label: &'static str,
    danger: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(theme::focus_ring)
        .h(px(32.))
        .px_4()
        .flex()
        .flex_shrink_0()
        .items_center()
        .rounded_full()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(danger, |d| {
            d.bg(gpui::rgb(0xc42b1c))
                .hover(|s| s.bg(gpui::rgb(0xd83b2c)))
        })
        .when(!danger, |d| {
            d.bg(theme::glass())
                .border_1()
                .border_color(theme::glass_border())
                .hover(|s| s.bg(theme::glass_hover()))
        })
        .child(label)
        .on_click(on_click)
}

/// Keyboard shortcuts shown in Settings, by area: (action, keys). Each entry in `keys` is one
/// way to do it; `+` joins keys pressed together.
/// An action and the keys that do it.
type Shortcut = (&'static str, &'static [&'static str]);

const SHORTCUTS: [(&str, &[Shortcut]); 3] = [
    (
        "Library",
        &[
            ("Search", &["Ctrl+K", "Ctrl+F"]),
            ("Home, Movies, TV Shows", &["Ctrl+1", "Ctrl+2", "Ctrl+3"]),
            ("Collapse or expand the sidebar", &["Ctrl+B"]),
            ("Back", &["Alt+←", "Backspace", "Esc"]),
            ("Forward", &["Alt+→"]),
            ("Move between items", &["←", "→", "↑", "↓", "Tab"]),
            ("Open the focused item", &["Enter", "Space"]),
            ("Previous or next featured title (Home)", &["←", "→"]),
        ],
    ),
    (
        "Search",
        &[
            ("Choose a result", &["↑", "↓"]),
            ("Open it", &["Enter"]),
            ("Show all results", &["Ctrl+Enter"]),
            ("Close", &["Esc"]),
        ],
    ),
    (
        "Player",
        &[
            ("Play or pause", &["Space", "K"]),
            ("Skip back or forward", &["←", "→", "J", "L"]),
            ("Back to the start", &["Home"]),
            ("Close a menu, then the player", &["Esc", "Backspace"]),
            ("Volume", &["Mouse wheel"]),
            ("Play or pause with the mouse", &["Right-click"]),
        ],
    ),
];

/// A key label, e.g. `Ctrl+K` drawn as two small keys.
fn key_combo(combo: &str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(combo.split('+').map(|key| {
            div()
                .min_w(px(24.))
                .h(px(24.))
                .px_1p5()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(rgba(0xffffff0f))
                .border_1()
                .border_color(theme::glass_border())
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text_secondary())
                .child(key.to_string())
        }))
        .into_any_element()
}

/// Rows for the Keyboard shortcuts section: a small heading per area, then one row per action.
pub(super) fn shortcut_rows() -> Vec<AnyElement> {
    let mut rows = Vec::new();
    for (area, shortcuts) in SHORTCUTS {
        rows.push(
            div()
                .px_5()
                .pt_4()
                .pb_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_tertiary())
                .child(area)
                .into_any_element(),
        );
        for (action, keys) in shortcuts {
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .px_5()
                    .py_2()
                    .child(div().flex_1().min_w_0().text_sm().child(*action))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_end()
                            .items_center()
                            .gap_2()
                            .children(keys.iter().map(|k| key_combo(k))),
                    )
                    .into_any_element(),
            );
        }
    }
    rows.push(div().h(px(12.)).into_any_element());
    rows
}

/// "Today", "Yesterday", "3 days ago"… from milliseconds since a title was played.
fn played_ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0..1 => "Just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => {
            let hours = minutes / 60;
            if hours == 1 {
                "1 hour ago".into()
            } else {
                format!("{hours} hours ago")
            }
        }
        1440..2880 => "Yesterday".into(),
        2880..20_160 => format!("{} days ago", minutes / 1440),
        20_160..86_400 => format!("{} weeks ago", minutes / 10_080),
        _ => {
            let months = (minutes / 43_200).max(2);
            if months >= 12 {
                format!(
                    "{} year{} ago",
                    months / 12,
                    if months >= 24 { "s" } else { "" }
                )
            } else {
                format!("{months} months ago")
            }
        }
    }
}
