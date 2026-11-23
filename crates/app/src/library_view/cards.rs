//! Library cards (posters, Continue Watching, episodes) as their own cached views.
//!
//! GPUI rebuilds, lays out and paints a whole view whenever it changes. With every card part
//! of the library view, a hover lift (a 200 ms animation) redrew every card on the page each
//! frame. As separate entities embedded with `cached`, a card only redraws when it is
//! notified: hovering one redraws that card, and the others are reused from the last frame.

use std::cell::RefCell;

use gpui::{Entity, StyleRefinement, WeakEntity};

use super::poster_drag::CarryArt;
use super::*;
use crate::image_cache::BoundedImageCache;

/// Space between a card's artwork and its caption.
const CARD_GAP: f32 = 12.;
/// Episode caption: the "EPISODE 3 · 52 min" line, the name, and the gaps between them.
const EPISODE_META_H: f32 = 18. + 4. + 24. + 4.;
/// Synopsis lines shown at rest and while hovered.
const SYNOPSIS_REST_LINES: f32 = 2.;
const SYNOPSIS_HOVER_LINES: usize = 8;
/// Rough characters per synopsis line at the card's width, to size the hovered card.
const SYNOPSIS_CHARS_PER_LINE: usize = 48;
/// Cards not drawn for this long are dropped on the next reload.
const CARD_IDLE: Duration = Duration::from_secs(120);

/// What a card shows; the card is notified only when this changes.
#[derive(Clone, PartialEq)]
pub(super) enum CardContent {
    Poster {
        item: LibraryItem,
        width: f32,
        watched: bool,
    },
    Continue(ContinueCard),
    Episode {
        show_title: String,
        accent: Option<u32>,
        file: MediaFile,
        progress: Option<Progress>,
        art: Option<EpisodeArt>,
    },
}

pub(super) struct Card {
    library: WeakEntity<LibraryView>,
    /// Where the card appears (a shelf, "grid", "collection"…).
    scope: &'static str,
    images: Entity<BoundedImageCache>,
    content: CardContent,
    hovered: bool,
    /// Bumped on every hover change so the lift animation restarts.
    hover_epoch: usize,
    /// Whether the card has been hovered at all; untouched cards skip the leave animation.
    touched: bool,
}

struct Slot {
    card: Entity<Card>,
    last_used: Instant,
}

/// Cards by where they appear (shelf, grid, show page, search) and what they show, kept
/// across frames so hover state survives.
#[derive(Default)]
pub(super) struct Cards(RefCell<HashMap<(&'static str, String), Slot>>);

impl LibraryView {
    /// Embeds the card at `slot` as a cached view of `w` x `h` (from its hover state), making
    /// it or updating its content first. `same` checks the existing content in place, so a
    /// card's content is only built (cloned) when it changed.
    fn cached_card(
        &self,
        slot: (&'static str, String),
        same: impl FnOnce(&CardContent) -> bool,
        content: impl FnOnce() -> CardContent,
        size: impl FnOnce(bool) -> (f32, f32),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scope = slot.0;
        let existing = self.cards.0.borrow_mut().get_mut(&slot).map(|s| {
            s.last_used = Instant::now();
            s.card.clone()
        });
        let card = match existing {
            Some(card) => {
                if !same(&card.read(cx).content) {
                    let content = content();
                    card.update(cx, |card, cx| {
                        card.content = content;
                        cx.notify();
                    });
                }
                card
            }
            None => {
                let (library, images, content) =
                    (cx.weak_entity(), self.image_cache.clone(), content());
                let card = cx.new(|_| Card {
                    library,
                    scope,
                    images,
                    content,
                    hovered: false,
                    hover_epoch: 0,
                    touched: false,
                });
                self.cards.0.borrow_mut().insert(
                    slot,
                    Slot {
                        card: card.clone(),
                        last_used: Instant::now(),
                    },
                );
                card
            }
        };
        // A cached view is laid out from this style, not measured: give its full size.
        let (w, h) = size(card.read(cx).hovered);
        card.cached(StyleRefinement::default().w(px(w)).h(px(h)).flex_shrink_0())
            .into_any_element()
    }

    /// Drops cards that haven't been drawn for a while (titles gone after a rescan, pages
    /// left long ago).
    pub(super) fn prune_cards(&self) {
        self.cards
            .0
            .borrow_mut()
            .retain(|_, slot| slot.last_used.elapsed() < CARD_IDLE);
    }

    /// A poster card for `item` at `width`. `scope` names where it appears, so the same title
    /// on a shelf and in a grid gets separate cards.
    pub(super) fn poster_card(
        &self,
        scope: &'static str,
        item: &LibraryItem,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let watched = self.is_watched(item);
        let slot = (
            scope,
            format!(
                "{}-{}-{:?}",
                item.key.kind as u8, item.key.title, item.key.year
            ),
        );
        self.cached_card(
            slot,
            |c| {
                matches!(c, CardContent::Poster { item: i, width: w, watched: d }
                    if *w == width && *d == watched && i == item)
            },
            || CardContent::Poster {
                item: item.clone(),
                width,
                watched,
            },
            |_| (width, width * 1.5 + CARD_GAP + CAPTION_H),
            cx,
        )
    }

    pub(super) fn continue_card(&self, card: &ContinueCard, cx: &mut Context<Self>) -> AnyElement {
        let slot = (
            "continue",
            format!("{:?}-{}", card.request.file_id, card.request.path),
        );
        self.cached_card(
            slot,
            |c| matches!(c, CardContent::Continue(existing) if existing == card),
            || CardContent::Continue(card.clone()),
            |_| (CONTINUE_W, CONTINUE_W * 9. / 16.),
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn episode_card(
        &self,
        scope: &'static str,
        show_title: &str,
        show_meta: &Metadata,
        file: &MediaFile,
        progress: Option<Progress>,
        art: Option<&EpisodeArt>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = show_meta.accent_rgb;
        let overview_len = art
            .and_then(|a| a.overview.as_ref())
            .map(|o| o.chars().count());
        self.cached_card(
            (scope, file.id.to_string()),
            |c| {
                matches!(c, CardContent::Episode { show_title: t, accent: a, file: f, progress: p, art: e }
                    if t == show_title && *a == accent && f == file && *p == progress && e.as_ref() == art)
            },
            || CardContent::Episode {
                show_title: show_title.to_string(),
                accent,
                file: file.clone(),
                progress,
                art: art.cloned(),
            },
            |hovered| {
                let lines = overview_len.map_or(0., |len| {
                    if hovered {
                        len.div_ceil(SYNOPSIS_CHARS_PER_LINE)
                            .clamp(SYNOPSIS_REST_LINES as usize, SYNOPSIS_HOVER_LINES)
                            as f32
                    } else {
                        SYNOPSIS_REST_LINES
                    }
                });
                let synopsis = if lines > 0. {
                    4. + lines * SYNOPSIS_LINE_H
                } else {
                    0.
                };
                (
                    EPISODE_CARD_W,
                    EPISODE_CARD_W * 9. / 16. + CARD_GAP + EPISODE_META_H + synopsis,
                )
            },
            cx,
        )
    }
}

impl Card {
    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.hovered != hovered {
            self.hovered = hovered;
            self.touched = true;
            self.hover_epoch += 1;
            cx.notify();
            // Episode cards change height with hover, which the library lays out.
            if matches!(self.content, CardContent::Episode { .. }) {
                self.library.update(cx, |_, cx| cx.notify()).ok();
            }
        }
    }

    /// Grows the artwork slightly with a deeper shadow while hovered, animating both ways.
    fn lift(&self, art: Div, w: f32, h: f32) -> AnyElement {
        let apply = move |el: Div, t: f32| {
            let (dw, dh) = (w * LIFT * t, h * LIFT * t);
            el.w(px(w + dw))
                .h(px(h + dh))
                .mx(px(-dw / 2.))
                .my(px(-dh / 2.))
                .shadow(theme::card_shadow(t))
        };
        let animation = || Animation::new(theme::motion(200)).with_easing(ease_out_quint());
        let epoch = self.hover_epoch;
        if self.hovered {
            art.with_animation(("card-lift-in", epoch), animation(), apply)
                .into_any_element()
        } else if self.touched {
            art.with_animation(("card-lift-out", epoch), animation(), move |el, t| {
                apply(el, 1. - t)
            })
            .into_any_element()
        } else {
            apply(art, 0.).into_any_element()
        }
    }

    /// Full-bleed artwork through the library's image cache (a card renders on its own,
    /// outside the library view's tree, so it names the cache).
    fn art(&self, path: Option<String>, radius: f32, max_width: u32) -> AnyElement {
        let inner: AnyElement = match path {
            Some(path) => art_image(path, max_width)
                .image_cache(&self.images)
                .size_full()
                .rounded(px(radius))
                .object_fit(ObjectFit::Cover)
                .with_fallback(|| div().into_any_element())
                .into_any_element(),
            None => div().into_any_element(),
        };
        div().absolute().inset_0().child(inner).into_any_element()
    }

    fn play(&self, request: PlaybackRequest, cx: &mut Context<Self>) {
        self.library
            .update(cx, |_, cx| cx.emit(LibraryEvent::Play(request)))
            .ok();
    }

    /// The "⋯" button on a hovered poster, opening its menu (watched, history, collections).
    fn poster_more_button(&self, key: TitleKey, cx: &mut Context<Self>) -> AnyElement {
        let scope = self.scope;
        div()
            .id("card-more")
            .absolute()
            .right(px(10.))
            .bottom(px(10.))
            .size(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x000000d9))
            .border_1()
            .border_color(rgba(0xffffff2e))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0x2a2a30f2)).border_color(rgba(0xffffff59)))
            .child(icon("icons/more.svg", 16., theme::text()))
            // The press belongs to the button: it doesn't pick the poster up or scroll the row.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |card, event: &ClickEvent, _, cx| {
                cx.stop_propagation();
                let position = event.position();
                let key = key.clone();
                card.library
                    .update(cx, |library, cx| {
                        let same = library.card_menu.as_ref().is_some_and(|m| m.key == key);
                        // A press outside an open menu closes it; don't reopen it at once.
                        let just_closed =
                            library.card_menu_closed.elapsed() < Duration::from_millis(200);
                        if same {
                            library.close_card_menu();
                            cx.notify();
                        } else if !just_closed {
                            library.open_card_menu(key, scope, position, cx);
                        }
                    })
                    .ok();
            }))
            .with_animation(
                ("card-more-in", self.hover_epoch),
                Animation::new(theme::motion(160)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }

    fn poster_body(
        &self,
        item: &LibraryItem,
        width: f32,
        watched: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let more = self
            .hovered
            .then(|| self.poster_more_button(item.key.clone(), cx));
        let meta = item.metadata.as_ref();
        let title: SharedString = item.display_title().to_string().into();
        let caption: SharedString = match item.key.kind {
            ItemKind::Movie => item
                .key
                .year
                .map(|y| y.to_string())
                .unwrap_or_default()
                .into(),
            ItemKind::Show => format!("{} episodes", item.file_count).into(),
        };
        let height = width * 1.5;
        let art_content: AnyElement = match meta.and_then(|m| m.poster_file.clone()) {
            Some(path) => self.art(Some(path), theme::RADIUS_CARD, ART_POSTER),
            None => div()
                .size_full()
                .p_4()
                .flex()
                .items_end()
                .font_family(theme::FONT_DISPLAY)
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .child(div().w_full().line_clamp(3).child(title.clone()))
                .into_any_element(),
        };
        let art = div()
            .relative()
            .rounded(px(theme::RADIUS_CARD))
            .overflow_hidden()
            .bg(theme::placeholder(&title, meta.and_then(|m| m.accent_rgb)))
            .child(art_content)
            // Small check badge in the corner for watched titles.
            .when(watched, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(10.))
                        .right(px(10.))
                        .size(px(24.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x000000b3))
                        .border_1()
                        .border_color(rgba(0xffffff26))
                        .child(icon("icons/check.svg", 12., theme::text())),
                )
            })
            .children(more);
        div()
            .w(px(width))
            .flex()
            .flex_col()
            .gap(px(CARD_GAP))
            .child(
                div()
                    .w(px(width))
                    .h(px(height))
                    .child(self.lift(art, width, height)),
            )
            .child(
                div()
                    .h(px(CAPTION_H))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .text_color(if self.hovered {
                                theme::text()
                            } else {
                                rgba(0xe8e8edff)
                            })
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_tertiary())
                            .child(caption),
                    ),
            )
    }

    fn continue_body(&self, card: &ContinueCard) -> Div {
        let height = CONTINUE_W * 9. / 16.;
        let art = div()
            .relative()
            .rounded(px(theme::RADIUS_CARD))
            .overflow_hidden()
            .bg(theme::placeholder(&card.title, card.accent))
            .child(self.art(card.art.clone(), theme::RADIUS_CARD, ART_CARD))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(rgba(0x000000d9), 0.),
                linear_color_stop(rgba(0x00000000), 0.6),
            )))
            .child(
                div()
                    .absolute()
                    .left_4()
                    .right_4()
                    .bottom_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(card.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgba(0xffffffb3))
                            .truncate()
                            .child(card.subtitle.clone()),
                    )
                    .when(card.fraction > 0., |d| {
                        d.child(div().mt_1().child(theme::progress_bar(card.fraction, 3.)))
                    }),
            );
        div()
            .w(px(CONTINUE_W))
            .h(px(height))
            .child(self.lift(art, CONTINUE_W, height))
    }

    fn episode_body(
        &self,
        show_title: &str,
        accent: Option<u32>,
        file: &MediaFile,
        progress: Option<Progress>,
        art: Option<&EpisodeArt>,
        cx: &mut Context<Self>,
    ) -> Div {
        let ParsedMedia::Episode {
            episode,
            episode_end,
            title,
            ..
        } = &file.media
        else {
            return div();
        };
        let number = match episode_end {
            Some(end) => format!("{episode}–{end}"),
            None => episode.to_string(),
        };
        // Prefer TMDB's episode name; fall back to the one parsed from the file name.
        let name = art
            .and_then(|a| a.name.clone())
            .or_else(|| title.clone())
            .unwrap_or_else(|| format!("Episode {number}"));
        let p = progress.unwrap_or_default();
        let still = art
            .filter(|a| a.source != ArtSource::None)
            .and_then(|a| a.still_file.clone());
        let loading = art.is_none();
        let width = EPISODE_CARD_W;
        let height = width * 9. / 16.;
        let hovered = self.hovered;

        let thumb = div()
            .relative()
            .rounded(px(theme::RADIUS_CARD))
            .overflow_hidden()
            .bg(theme::placeholder(show_title, accent))
            .child(self.art(still.clone(), theme::RADIUS_CARD, ART_STILL))
            // Without a still, show the episode number large on the show's colour.
            .when(still.is_none(), |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(theme::FONT_DISPLAY)
                        .text_size(px(56.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgba(0xffffff40))
                        .child(if loading {
                            String::new()
                        } else {
                            number.clone()
                        }),
                )
            })
            // Bottom scrim so the progress bar and badges read on bright stills.
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(rgba(0x000000a6), 0.),
                linear_color_stop(rgba(0x00000000), 0.45),
            )))
            .when(hovered, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(52.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgba(0x000000b3))
                                .border_1()
                                .border_color(theme::glass_border())
                                .child(icon("icons/play.svg", 22., theme::text())),
                        ),
                )
            })
            // Watched check, always shown once watched and offered on hover otherwise; clicking
            // it toggles the episode without playing it.
            .when(p.watched || hovered, |d| {
                let (file_id, watched) = (file.id, p.watched);
                d.child(
                    div()
                        .id("episode-watched")
                        .absolute()
                        .top_2()
                        .right_2()
                        .size(px(28.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x000000cc))
                        .border_1()
                        .border_color(rgba(0xffffff26))
                        .hover(|s| s.bg(rgba(0x2a2a33f0)))
                        .child(icon(
                            "icons/check.svg",
                            14.,
                            if watched {
                                theme::text()
                            } else {
                                theme::text_tertiary()
                            },
                        ))
                        .tooltip(theme::text_tooltip(if watched {
                            "Mark as unwatched"
                        } else {
                            "Mark as watched"
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.library
                                .update(cx, |library, cx| {
                                    library.mark_watched(vec![file_id], !watched, cx)
                                })
                                .ok();
                        })),
                )
            })
            .when(p.resume_at().is_some(), |d| {
                d.child(
                    div()
                        .absolute()
                        .left_3()
                        .right_3()
                        .bottom_3()
                        .child(theme::progress_bar(p.fraction(), 3.)),
                )
            });

        let mut details = Vec::new();
        if let Some(runtime) = art.and_then(|a| a.runtime_min) {
            details.push(format!("{runtime} min"));
        } else if p.duration > 0. {
            details.push(format_duration(p.duration));
        }
        if p.resume_at().is_some() {
            details.push(remaining(&p));
        }
        let overview = art.and_then(|a| a.overview.clone());
        let synopsis_lines = if hovered {
            SYNOPSIS_HOVER_LINES as f32
        } else {
            SYNOPSIS_REST_LINES
        };

        div()
            .w(px(width))
            .flex()
            .flex_col()
            .gap(px(CARD_GAP))
            .child(
                div()
                    .w(px(width))
                    .h(px(height))
                    .child(self.lift(thumb, width, height)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text_tertiary())
                            .child(format!("EPISODE {number}"))
                            .children((!details.is_empty()).then(|| {
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("·  {}", details.join("  ·  ")))
                            })),
                    )
                    .child(
                        div()
                            .h(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .text_color(if p.watched {
                                theme::text_secondary()
                            } else {
                                theme::text()
                            })
                            .child(name),
                    )
                    .children(overview.map(|o| {
                        div()
                            .text_sm()
                            .line_height(px(SYNOPSIS_LINE_H))
                            // Two lines at rest; hovering the card reveals most of the synopsis.
                            // Clipped by height rather than `line_clamp`: GPUI caches wrapped
                            // text without the clamp, so a changing clamp keeps the old layout.
                            .overflow_hidden()
                            .max_h(px(SYNOPSIS_LINE_H * synopsis_lines))
                            .text_color(if hovered {
                                theme::text()
                            } else {
                                theme::text_secondary()
                            })
                            .child(o)
                    })),
            )
    }
}

type CardClick = Box<dyn Fn(&mut Card, &mut Context<Card>)>;

impl Render for Card {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (body, on_click): (Div, CardClick) = match &self.content {
            CardContent::Poster {
                item,
                width,
                watched,
            } => {
                let key = item.key.clone();
                (
                    self.poster_body(&item.clone(), *width, *watched, cx),
                    Box::new(move |card, cx| {
                        card.library
                            .update(cx, |library, cx| library.open_key(&key, cx))
                            .ok();
                    }),
                )
            }
            CardContent::Continue(card) => {
                let request = card.request.clone();
                (
                    self.continue_body(card),
                    Box::new(move |card, cx| card.play(request.clone(), cx)),
                )
            }
            CardContent::Episode {
                show_title,
                accent,
                file,
                progress,
                art,
            } => {
                let request = episode_request(show_title, file, &progress.unwrap_or_default());
                let (show_title, accent, file, progress, art) = (
                    show_title.clone(),
                    *accent,
                    file.clone(),
                    *progress,
                    art.clone(),
                );
                (
                    self.episode_body(&show_title, accent, &file, progress, art.as_ref(), cx),
                    Box::new(move |card, cx| card.play(request.clone(), cx)),
                )
            }
        };
        let press = match &self.content {
            CardContent::Poster { item, width, .. } => {
                let art = CarryArt {
                    poster: item.metadata.as_ref().and_then(|m| m.poster_file.clone()),
                    title: item.display_title().to_string().into(),
                    accent: item.metadata.as_ref().and_then(|m| m.accent_rgb),
                };
                Some((item.key.clone(), art, *width))
            }
            _ => None,
        };
        let scope = self.scope;
        div()
            .id("card")
            .tab_index(0)
            // Pressing a poster may become a drag onto a collection (see poster_drag.rs).
            .when_some(press, |d, (key, art, width)| {
                d.on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |card, event: &gpui::MouseDownEvent, _, cx| {
                        let (key, art) = (key.clone(), art.clone());
                        card.library
                            .update(cx, |library, cx| {
                                library.press_poster(key, art, scope, event.position, width, cx)
                            })
                            .ok();
                    }),
                )
            })
            .rounded(px(theme::RADIUS_CARD + 2.))
            .focus_visible(theme::focus_ring)
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.set_hovered(*hovered, cx)))
            .on_click(cx.listener(move |this, _, _, cx| {
                // Releasing after dragging the row lands on a card; that isn't a click on it.
                let dragging = this
                    .library
                    .upgrade()
                    .is_some_and(|library| library.read(cx).click_ends_drag());
                if !dragging {
                    on_click(this, cx);
                }
            }))
            .child(body)
    }
}
