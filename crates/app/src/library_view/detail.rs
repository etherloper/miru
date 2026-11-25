//! Movie and show detail pages.

use super::*;

impl LibraryView {
    // ---- detail pages ----------------------------------------------------------------------

    pub(super) fn render_movie(
        &self,
        item: &LibraryItem,
        progress: Option<Progress>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let meta = item.metadata.clone().unwrap_or_default();
        let mut facts = Vec::new();
        if let Some(year) = meta.release_year.or(item.key.year) {
            facts.push(year.to_string());
        }
        if let Some(minutes) = meta.runtime_min {
            facts.push(format_duration(f64::from(minutes) * 60.));
        } else if let Some(p) = progress.filter(|p| p.duration > 0.) {
            facts.push(format_duration(p.duration));
        }
        if item.file_count > 1 {
            facts.push(format!("{} versions", item.file_count));
        }
        let watched = progress.is_some_and(|p| p.watched);

        let resume_at = progress.and_then(|p| p.resume_at());
        let (play_item, restart_item) = (item.clone(), item.clone());
        let mut actions: Vec<AnyElement> = Vec::new();
        match resume_at {
            Some(at) => {
                actions.push(
                    primary_button(
                        "movie-resume",
                        Some("icons/play.svg"),
                        format!("Resume from {}", format_time(at)),
                        cx.listener(move |this, _, _, cx| this.play_movie(&play_item, true, cx)),
                    )
                    .into_any_element(),
                );
                actions.push(
                    glass_button(
                        "movie-restart",
                        Some("icons/restart.svg"),
                        "Start Over",
                        cx.listener(move |this, _, _, cx| {
                            this.play_movie(&restart_item, false, cx)
                        }),
                    )
                    .into_any_element(),
                );
            }
            None => actions.push(
                primary_button(
                    "movie-play",
                    Some("icons/play.svg"),
                    if watched { "Play Again" } else { "Play" },
                    cx.listener(move |this, _, _, cx| this.play_movie(&play_item, false, cx)),
                )
                .into_any_element(),
            ),
        }

        let (fix_item, file_id) = (item.clone(), item.file_id);
        actions.push(self.more_menu(
            vec![
                MenuEntry::new(
                    if watched {
                        "Mark as unwatched"
                    } else {
                        "Mark as watched"
                    },
                    if watched {
                        "icons/circle.svg"
                    } else {
                        "icons/check.svg"
                    },
                    move |this, _, cx| this.mark_watched(vec![file_id], !watched, cx),
                ),
                MenuEntry::new("Fix match…", "icons/pencil.svg", move |this, _, cx| {
                    this.open_fix_match(fix_item.clone(), cx)
                }),
            ],
            Some(&item.key),
            cx,
        ));

        let extra = resume_at.zip(progress).map(|(_, p)| {
            div()
                .w(px(300.))
                .flex()
                .items_center()
                .gap_3()
                .child(div().flex_1().child(theme::progress_bar(p.fraction(), 3.)))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_secondary())
                        .child(remaining(&p)),
                )
                .into_any_element()
        });

        div()
            .id("movie-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.detail_scroll)
            .child(self.detail_hero(
                item,
                &meta,
                Tab::Movies,
                facts,
                watched,
                extra,
                actions,
                window,
                cx,
            ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_show(
        &self,
        item: &LibraryItem,
        episodes: &[MediaFile],
        progress: &HashMap<i64, Progress>,
        season: Option<u16>,
        art: &HashMap<i64, EpisodeArt>,
        up_next: Option<i64>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let meta = item.metadata.clone().unwrap_or_default();
        let show_title = item.display_title().to_string();
        let seasons: Vec<u16> = episodes
            .iter()
            .filter_map(|e| season_of(&e.media))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let season = season
            .filter(|s| seasons.contains(s))
            .or_else(|| seasons.first().copied());
        let watched = episodes
            .iter()
            .filter(|e| progress.get(&e.id).is_some_and(|p| p.watched))
            .count();
        let mut facts: Vec<String> = meta
            .release_year
            .map(|y| y.to_string())
            .into_iter()
            .collect();
        facts.extend([
            format!(
                "{} season{}",
                seasons.len(),
                if seasons.len() == 1 { "" } else { "s" }
            ),
            format!("{} episodes", episodes.len()),
        ]);

        let mut actions: Vec<AnyElement> = Vec::new();
        if let Some(next) = offered_episode(episodes, progress, up_next) {
            let p = progress.get(&next.id).copied().unwrap_or_default();
            let short = short_episode(&next.media);
            let label = match p.resume_at() {
                Some(_) => format!("Resume {short}"),
                None => format!("Play {short}"),
            };
            let request = episode_request(&show_title, next, &p);
            actions.push(
                primary_button(
                    "show-next",
                    Some("icons/play.svg"),
                    label,
                    cx.listener(move |_, _, _, cx| cx.emit(LibraryEvent::Play(request.clone()))),
                )
                .into_any_element(),
            );
        }

        let fix_item = item.clone();
        let all_ids: Vec<i64> = episodes.iter().map(|e| e.id).collect();
        let all_watched = !episodes.is_empty() && watched == episodes.len();
        let mut entries = vec![MenuEntry::new(
            if all_watched {
                "Mark show as unwatched"
            } else {
                "Mark show as watched"
            },
            if all_watched {
                "icons/circle.svg"
            } else {
                "icons/check.svg"
            },
            move |this, _, cx| this.mark_watched(all_ids.clone(), !all_watched, cx),
        )];
        if let Some(s) = season.filter(|_| seasons.len() > 1) {
            let season_ids: Vec<i64> = episodes
                .iter()
                .filter(|e| season_of(&e.media) == Some(s))
                .map(|e| e.id)
                .collect();
            let season_watched = season_ids
                .iter()
                .all(|id| progress.get(id).is_some_and(|p| p.watched));
            entries.push(MenuEntry::new(
                if season_watched {
                    format!("Mark Season {s} as unwatched")
                } else {
                    format!("Mark Season {s} as watched")
                },
                if season_watched {
                    "icons/circle.svg"
                } else {
                    "icons/check.svg"
                },
                move |this, _, cx| this.mark_watched(season_ids.clone(), !season_watched, cx),
            ));
        }
        entries.push(MenuEntry::new(
            "Fix match…",
            "icons/pencil.svg",
            move |this, _, cx| this.open_fix_match(fix_item.clone(), cx),
        ));
        actions.push(self.more_menu(entries, Some(&item.key), cx));

        let season_tabs = (seasons.len() > 1).then(|| {
            div().flex().gap_2().children(seasons.iter().map(|&s| {
                let selected = Some(s) == season;
                div()
                    .id(("season", s as usize))
                    .tab_index(0)
                    .focus_visible(theme::focus_ring)
                    .h(px(34.))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme::text()).text_color(theme::bg()))
                    .when(!selected, |d| {
                        d.bg(theme::glass())
                            .text_color(theme::text_secondary())
                            .hover(|h| h.bg(theme::glass_hover()))
                    })
                    .child(format!("Season {s}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Page::Show { season, .. } = &mut this.page {
                            *season = Some(s);
                            cx.notify();
                        }
                    }))
            }))
        });

        let rows = episodes
            .iter()
            .enumerate()
            .filter(|(_, e)| season_of(&e.media) == season)
            .map(|(_, file)| {
                self.episode_card(
                    "episodes",
                    &show_title,
                    &meta,
                    file,
                    progress.get(&file.id).copied(),
                    art.get(&file.id),
                    cx,
                )
            })
            .collect::<Vec<_>>();

        div()
            .id("show-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.detail_scroll)
            .child(self.detail_hero(
                item,
                &meta,
                Tab::Shows,
                facts,
                false,
                None,
                actions,
                window,
                cx,
            ))
            .child(
                div()
                    // Rises into the bottom of the hero, over its faded artwork.
                    .relative()
                    .mt(px(-EPISODES_OVERLAP))
                    .pb_8()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px(px(PAGE_PADDING))
                            .flex()
                            .items_center()
                            .gap_6()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .text_size(px(21.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Episodes"),
                            )
                            .children(season_tabs),
                    )
                    .child(self.shelf_row("shelf-episodes", rows, cx)),
            )
    }

    /// Full-bleed header shared by movie and show pages. It grows with its content, so long
    /// synopses are shown in full rather than clipped.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn detail_hero(
        &self,
        item: &LibraryItem,
        meta: &Metadata,
        back_to: Tab,
        facts: Vec<String>,
        watched: bool,
        extra: Option<AnyElement>,
        actions: Vec<AnyElement>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let viewport_h = f32::from(window.viewport_size().height);
        // A movie page is just the hero, so it fills the window (no empty band below). A show
        // page leaves room under it for the episode row without scrolling.
        let is_show = back_to == Tab::Shows;
        let min_height = if is_show {
            // The smaller of 72% of the window and what leaves the episode row in view, plus the
            // band the row overlaps (see `EPISODES_OVERLAP`).
            (viewport_h * 0.72)
                .min(viewport_h - EPISODES_ROW_H)
                .max(460.)
                + EPISODES_OVERLAP
        } else {
            viewport_h
        };
        let backdrop = pick_backdrop(meta);
        // Without a backdrop, feature the poster on the right instead.
        let poster = backdrop
            .is_none()
            .then(|| meta.poster_file.clone())
            .flatten()
            .map(|path| {
                div()
                    .absolute()
                    .right(px(PAGE_PADDING + 40.))
                    .top(px(NAV_HEIGHT + 40.))
                    .w(px(240.))
                    .h(px(360.))
                    .rounded(px(theme::RADIUS_CARD))
                    .overflow_hidden()
                    .shadow(theme::card_shadow(1.))
                    .child(
                        art_image(path, ART_POSTER)
                            .size_full()
                            .rounded(px(theme::RADIUS_CARD))
                            .object_fit(ObjectFit::Cover),
                    )
            });

        // Year · runtime · genres, then age-rating and audience-rating chips.
        let mut line = facts;
        if !meta.genres.is_empty() {
            line.push(
                meta.genres
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        let chip = |content: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .h(px(22.))
                .px_2()
                .rounded(px(6.))
                .border_1()
                .border_color(rgba(0xffffff4d))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(content)
        };
        // The chosen sites' ratings from MDBList; otherwise TMDB's audience score.
        let mut rating = self.rating_badges(&item.key);
        if rating.is_empty()
            && let Some(r) = meta.rating_tenths
        {
            rating.push(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(26.))
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child(div().text_color(gpui::rgb(0xf5c518)).child("★"))
                    .child(format!("{:.1}", f32::from(r) / 10.))
                    .into_any_element(),
            );
        }
        // "Starring" with the first few names, then a link to the full cast sheet.
        let cast_item = item.clone();
        let starring = (!meta.cast.is_empty()).then(|| {
            div()
                .max_w(px(680.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_3()
                .gap_y_1()
                .text_sm()
                .text_color(theme::text_secondary())
                .child(format!(
                    "Starring {}",
                    meta.cast
                        .iter()
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .child(
                    div()
                        .id("view-cast")
                        .tab_index(0)
                        .focus_visible(theme::focus_ring)
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(px(6.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::accent()))
                        .child("View full cast")
                        .child(icon(
                            "icons/chevron-right.svg",
                            12.,
                            theme::text_secondary(),
                        ))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_cast(&cast_item, cx)),
                        ),
                )
        });

        div()
            .relative()
            .w_full()
            .min_h(px(min_height))
            .flex()
            .flex_col()
            .justify_end()
            .overflow_hidden()
            .bg(theme::placeholder(&item.key.title, meta.accent_rgb))
            .child(art_layer(backdrop, 0., ART_BACKDROP))
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(rgba(0x000000f2), 0.),
                linear_color_stop(rgba(0x00000000), 0.7),
            )))
            // Bottom fade into the page, eased: GPUI gradients have two stops, so a short solid
            // fade and a long soft one are layered, which avoids the hard line a single linear
            // fade leaves where it turns solid.
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::bg(), 0.),
                linear_color_stop(rgba(0x00000000), 0.28),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(rgba(0x000000b3), 0.),
                linear_color_stop(rgba(0x00000000), 0.62),
            )))
            .children(poster)
            .child(
                div()
                    .absolute()
                    .top(px(NAV_HEIGHT + 8.))
                    .left(px(PAGE_PADDING))
                    .child(round_button(
                        "detail-back",
                        "icons/chevron-left.svg",
                        40.,
                        cx.listener(move |this, _, _, cx| {
                            if this.back_stack.is_empty() {
                                this.set_tab(back_to, cx);
                            } else {
                                this.go_back(cx);
                            }
                        }),
                    )),
            )
            .child(
                div()
                    .relative()
                    .pt(px(NAV_HEIGHT + if is_show { 80. } else { 96. }))
                    .px(px(PAGE_PADDING))
                    // A movie page keeps room below its buttons for the ⋯ menu to open into.
                    .pb(px(if is_show {
                        36. + EPISODES_OVERLAP
                    } else {
                        150.
                    }))
                    .max_w(px(780.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.title_art(item, meta, 150., 520., 56.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(theme::text_secondary())
                            .child(line.join("  ·  "))
                            .when(watched, |d| {
                                d.child(chip(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(icon("icons/check.svg", 12., theme::text()))
                                        .child("Watched")
                                        .into_any_element(),
                                ))
                            }),
                    )
                    // Ratings from other sites get a row of their own under the details.
                    .when(!rating.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x_4()
                                .gap_y_2()
                                .children(rating),
                        )
                    })
                    .children(meta.overview.clone().map(|o| {
                        div()
                            .max_w(px(680.))
                            .text_color(rgba(0xe8e8edff))
                            .line_height(px(24.))
                            .child(o)
                    }))
                    .children(starring)
                    .children(extra)
                    .child(div().mt_3().flex().items_center().gap_3().children(actions)),
            )
    }

    /// The title's logo when one was downloaded (fitted inside `max_h` x `max_w` by its real
    /// proportions), otherwise the title as large display text.
    pub(super) fn title_art(
        &self,
        item: &LibraryItem,
        meta: &Metadata,
        max_h: f32,
        max_w: f32,
        text_size: f32,
    ) -> AnyElement {
        if let Some(logo) = meta.logo_file.clone()
            && let Some((w, h)) = self.image_size(&logo)
        {
            let aspect = w as f32 / h.max(1) as f32;
            let (mut width, mut height) = (max_h * aspect, max_h);
            if width > max_w {
                width = max_w;
                height = max_w / aspect;
            }
            let title = item.display_title().to_string();
            return art_image(logo, ART_LOGO)
                .w(px(width))
                .h(px(height))
                .object_fit(ObjectFit::Contain)
                .with_fallback(move || div().child(title.clone()).into_any_element())
                .into_any_element();
        }
        div()
            .font_family(theme::FONT_DISPLAY)
            .text_size(px(text_size))
            .line_height(px(text_size * 1.08))
            .font_weight(FontWeight::BOLD)
            .child(item.display_title().to_string())
            .into_any_element()
    }

    /// Pixel size of a local image, read from its header once and cached.
    pub(super) fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        if let Some(size) = self.image_sizes.borrow().get(path) {
            return *size;
        }
        let size = image::image_dimensions(path).ok();
        self.image_sizes.borrow_mut().insert(path.to_string(), size);
        size
    }
}

/// One of a title's backdrops, chosen per launch so the art varies between sessions but
/// stays put while the app is open.
pub(super) fn pick_backdrop(meta: &Metadata) -> Option<String> {
    use std::hash::{Hash, Hasher};
    static LAUNCH: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let launch = *LAUNCH.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    });
    let options: Vec<&String> = if meta.backdrops.is_empty() {
        meta.backdrop_file.iter().collect()
    } else {
        meta.backdrops.iter().collect()
    };
    if options.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (launch, meta.tmdb_id, &meta.poster_file).hash(&mut hasher);
    options
        .get((hasher.finish() % options.len() as u64) as usize)
        .map(|s| (*s).clone())
}

pub(super) fn hero_art(item: &LibraryItem) -> AnyElement {
    let meta = item.metadata.clone().unwrap_or_default();
    match (pick_backdrop(&meta), meta.poster_file.clone()) {
        (Some(backdrop), _) => art_layer(Some(backdrop), 0., ART_BACKDROP),
        // No backdrop: accent wash with the poster floating on the right.
        (None, Some(poster)) => div()
            .absolute()
            .inset_0()
            .bg(theme::placeholder(&item.key.title, meta.accent_rgb))
            .child(
                div()
                    .absolute()
                    .right(px(PAGE_PADDING + 60.))
                    .top(px(NAV_HEIGHT + 36.))
                    .w(px(260.))
                    .h(px(390.))
                    .rounded(px(theme::RADIUS_CARD))
                    .overflow_hidden()
                    .shadow(theme::card_shadow(1.))
                    .child(
                        art_image(poster, ART_POSTER)
                            .size_full()
                            .rounded(px(theme::RADIUS_CARD))
                            .object_fit(ObjectFit::Cover),
                    ),
            )
            .into_any_element(),
        (None, None) => div()
            .absolute()
            .inset_0()
            .bg(theme::placeholder(&item.key.title, meta.accent_rgb))
            .into_any_element(),
    }
}
