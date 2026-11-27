//! Episodes as a list of rows on show pages: an alternative to the row of cards that's easier
//! to scan for seasons with many episodes. The choice is kept in Settings.

use super::*;

/// Thumbnail size in a row (16:9).
const THUMB_W: f32 = 208.;
const THUMB_H: f32 = THUMB_W * 9. / 16.;

impl LibraryView {
    /// The cards / list switch beside the Episodes heading.
    pub(super) fn episode_view_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let list = self.settings.episode_list;
        let option =
            |id: &'static str, path: &'static str, tip: &'static str, on: bool, value: bool| {
                div()
                    .id(id)
                    .tab_index(0)
                    .focus_visible(theme::focus_ring)
                    .size(px(32.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .map(|d| {
                        if on {
                            d.bg(rgba(0xffffff2e))
                        } else {
                            d.hover(|s| s.bg(rgba(0xffffff14)))
                        }
                    })
                    .child(icon(
                        path,
                        16.,
                        if on {
                            theme::text()
                        } else {
                            theme::text_secondary()
                        },
                    ))
                    .tooltip(theme::text_tooltip(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.settings.episode_list != value {
                            this.update_settings(|s| s.episode_list = value, cx);
                        }
                    }))
            };
        div()
            .flex()
            .flex_shrink_0()
            .gap_0p5()
            .p(px(3.))
            .rounded_full()
            .bg(theme::glass())
            .border_1()
            .border_color(theme::glass_border())
            .child(option(
                "episodes-as-cards",
                "icons/cards.svg",
                "Show as cards",
                !list,
                false,
            ))
            .child(option(
                "episodes-as-list",
                "icons/list.svg",
                "Show as a list",
                list,
                true,
            ))
            .into_any_element()
    }

    /// One episode as a row: thumbnail, number and name, length, synopsis, and the watched
    /// check. Clicking the row plays it.
    pub(super) fn episode_row(
        &self,
        show_title: &str,
        meta: &Metadata,
        file: &MediaFile,
        progress: Option<Progress>,
        art: Option<&EpisodeArt>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ParsedMedia::Episode {
            episode,
            episode_end,
            title,
            ..
        } = &file.media
        else {
            return theme::nothing();
        };
        let number = match episode_end {
            Some(end) => format!("{episode}–{end}"),
            None => episode.to_string(),
        };
        let name = art
            .and_then(|a| a.name.clone())
            .or_else(|| title.clone())
            .unwrap_or_else(|| format!("Episode {number}"));
        let p = progress.unwrap_or_default();
        let still = art
            .filter(|a| a.source != ArtSource::None)
            .and_then(|a| a.still_file.clone());
        let overview = art.and_then(|a| a.overview.clone());
        let mut details = Vec::new();
        if let Some(runtime) = art.and_then(|a| a.runtime_min) {
            details.push(format!("{runtime} min"));
        } else if p.duration > 0. {
            details.push(format_duration(p.duration));
        }
        if p.resume_at().is_some() {
            details.push(remaining(&p));
        }
        let request = episode_request(show_title, file, &p);
        let (file_id, watched) = (file.id, p.watched);

        let thumb = div()
            .relative()
            .w(px(THUMB_W))
            .h(px(THUMB_H))
            .flex_shrink_0()
            .rounded(px(theme::RADIUS_CARD))
            .overflow_hidden()
            .bg(theme::placeholder(show_title, meta.accent_rgb))
            .child(art_layer(still.clone(), theme::RADIUS_CARD, ART_STILL / 2))
            .when(still.is_none() && art.is_some(), |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(theme::FONT_DISPLAY)
                        .text_size(px(36.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgba(0xffffff40))
                        .child(number.clone()),
                )
            })
            // Play badge while the row is hovered.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x00000059))
                    .opacity(0.)
                    .group_hover("episode-row", |s| s.opacity(1.))
                    .child(
                        div()
                            .size(px(40.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba(0x000000b3))
                            .border_1()
                            .border_color(theme::glass_border())
                            .child(icon("icons/play.svg", 16., theme::text())),
                    ),
            )
            .when(p.resume_at().is_some(), |d| {
                d.child(
                    div()
                        .absolute()
                        .left_2()
                        .right_2()
                        .bottom_2()
                        .child(theme::progress_bar(p.fraction(), 3.)),
                )
            });

        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_3()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text_tertiary())
                            .child(number),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if watched {
                                theme::text_secondary()
                            } else {
                                theme::text()
                            })
                            .child(name),
                    ),
            )
            .when(!details.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text_tertiary())
                        .child(details.join("  ·  ")),
                )
            })
            .children(overview.map(|o| {
                div()
                    .pt_1()
                    .text_sm()
                    .line_height(px(SYNOPSIS_LINE_H))
                    .line_clamp(2)
                    .text_color(theme::text_secondary())
                    .child(o)
            }));

        let check = div()
            .id("episode-row-watched")
            .size(px(32.))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .cursor_pointer()
            .map(|d| {
                if watched {
                    d.bg(rgba(0xffffff1f)).border_color(rgba(0xffffff26))
                } else {
                    d.border_color(rgba(0xffffff26))
                        .opacity(0.)
                        .group_hover("episode-row", |s| s.opacity(1.))
                }
            })
            .hover(|s| s.bg(rgba(0xffffff2e)))
            .child(icon(
                "icons/check.svg",
                14.,
                if watched {
                    theme::text()
                } else {
                    theme::text_secondary()
                },
            ))
            .tooltip(theme::text_tooltip(if watched {
                "Mark as unwatched"
            } else {
                "Mark as watched"
            }))
            // The press belongs to the check, not the row (which would play the episode).
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.mark_watched(vec![file_id], !watched, cx);
            }));

        div()
            .id(("episode-row", file.id as u64))
            .group("episode-row")
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .flex()
            .items_center()
            .gap_5()
            .p_2()
            .pr_4()
            .rounded(px(theme::RADIUS_CARD + 4.))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xffffff0a)))
            .child(thumb)
            .child(text)
            .child(check)
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(LibraryEvent::Play(request.clone()))))
            .into_any_element()
    }
}
