//! Full cast pane: a sheet from the right listing a title's cast with photos. Choosing someone
//! shows the other titles in the library they appear in.

use miru_core::library::CastMember;

use super::*;

const SHEET_W: f32 = 420.;
const AVATAR: f32 = 52.;
const PERSON_AVATAR: f32 = 96.;
const CREDIT_POSTER_W: f32 = 112.;

pub(super) struct CastPane {
    /// The title whose cast this is; `None` for a person opened on their own (from search).
    key: Option<TitleKey>,
    /// The title's name, or the search that found the person.
    title: String,
    members: Vec<CastMember>,
    /// Cached photo files by person id; filled in as downloads finish.
    photos: HashMap<i64, String>,
    /// Other library titles per person id (for the "In N more" hint).
    elsewhere: HashMap<i64, usize>,
    person: Option<PersonView>,
}

impl CastPane {
    pub(super) fn showing_person(&self) -> bool {
        self.person.is_some()
    }
}

struct PersonView {
    member: CastMember,
    /// Library titles they appear in, with the character they play.
    credits: Vec<(LibraryItem, Option<String>)>,
    known_for: KnownFor,
}

/// A title someone is known for on TMDB, with its cached poster thumbnail.
pub(super) struct KnownTitle {
    credit: miru_metadata::KnownCredit,
    thumb: Option<String>,
}

/// How many "known for" titles to fetch; those already in the library are then left out.
const KNOWN_FOR_LIMIT: usize = 12;

enum KnownFor {
    Loading,
    Ready(Rc<Vec<KnownTitle>>),
    /// No TMDB key, or the request failed: the section is left out.
    Unavailable,
}

impl LibraryView {
    pub(super) fn open_cast(&mut self, item: &LibraryItem, cx: &mut Context<Self>) {
        let meta = item.metadata.clone().unwrap_or_default();
        let mut members = self.db.cast(&item.key).unwrap_or_else(|e| {
            tracing::error!("reading cast: {e}");
            Vec::new()
        });
        // Directors and creators lead the list, their job where a character would be.
        let crew = self.db.crew(&item.key).unwrap_or_else(|e| {
            tracing::error!("reading crew: {e}");
            Vec::new()
        });
        // Titles not refreshed since the full cast was added only have names.
        if members.is_empty() {
            members = meta
                .cast
                .iter()
                .map(|name| CastMember {
                    name: name.clone(),
                    ..Default::default()
                })
                .collect();
        }
        members.splice(0..0, crew);
        let elsewhere = members
            .iter()
            .filter(|m| m.person_id != 0)
            .map(|m| {
                let others = self
                    .library_credits(m.person_id)
                    .iter()
                    .filter(|(other, _)| other.key != item.key)
                    .count();
                (m.person_id, others)
            })
            .collect();
        let artwork = self.paths.artwork.clone();
        let photos: HashMap<i64, String> = members
            .iter()
            .filter_map(|m| {
                let file = artwork.join(format!("person-{}.jpg", m.person_id));
                file.is_file()
                    .then(|| file.to_str().map(|f| (m.person_id, f.to_string())))
                    .flatten()
            })
            .collect();
        let missing: Vec<(i64, String)> = members
            .iter()
            .filter(|m| m.person_id != 0 && !photos.contains_key(&m.person_id))
            .filter_map(|m| Some((m.person_id, m.profile_path.clone()?)))
            .collect();
        self.cast_pane = Some(CastPane {
            key: Some(item.key.clone()),
            title: item.display_title().to_string(),
            members,
            photos,
            elsewhere,
            person: None,
        });
        self.cast_list_scroll.reset();
        self.cast_epoch += 1;
        cx.notify();

        let (Some(tmdb), false) = (self.tmdb.clone(), missing.is_empty()) else {
            return;
        };
        let (paths, key) = (self.paths.clone(), item.key.clone());
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move { library_sync::cast_photos(&paths, &tmdb, &missing) })
                .await;
            this.update(cx, |view, cx| {
                let Some(pane) = view
                    .cast_pane
                    .as_mut()
                    .filter(|p| p.key.as_ref() == Some(&key))
                else {
                    return;
                };
                match fetched {
                    Ok(photos) => pane.photos.extend(
                        photos
                            .into_iter()
                            .filter_map(|(id, path)| Some((id, path.to_str()?.to_string()))),
                    ),
                    Err(e) => tracing::warn!("cast photos: {e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn close_cast(&mut self, cx: &mut Context<Self>) {
        if self.cast_pane.take().is_some() {
            cx.notify();
        }
    }

    /// Esc: from a person back to the cast list, then closed.
    pub(super) fn cast_back(&mut self, cx: &mut Context<Self>) {
        match &mut self.cast_pane {
            Some(pane) if pane.person.is_some() && pane.key.is_some() => pane.person = None,
            Some(_) => self.cast_pane = None,
            None => return,
        }
        cx.notify();
    }

    /// Opens `item`'s cast sheet on `member` (a name in "Directed by"); Back shows the cast.
    pub(super) fn open_cast_person(
        &mut self,
        item: &LibraryItem,
        member: CastMember,
        cx: &mut Context<Self>,
    ) {
        self.open_cast(item, cx);
        self.open_person(member, cx);
    }

    /// Opens the sheet on one person, outside any title's cast (a person found by search).
    pub(super) fn open_person_alone(&mut self, member: CastMember, cx: &mut Context<Self>) {
        let mut photos = HashMap::new();
        let file = self
            .paths
            .artwork
            .join(format!("person-{}.jpg", member.person_id));
        if let Some(path) = file.is_file().then(|| file.to_str()).flatten() {
            photos.insert(member.person_id, path.to_string());
        }
        let missing: Vec<(i64, String)> = member
            .profile_path
            .clone()
            .filter(|_| photos.is_empty())
            .map(|p| (member.person_id, p))
            .into_iter()
            .collect();
        let person_id = member.person_id;
        self.cast_pane = Some(CastPane {
            key: None,
            title: format!("“{}”", self.search_query),
            members: Vec::new(),
            photos,
            elsewhere: HashMap::new(),
            person: None,
        });
        self.cast_epoch += 1;
        self.open_person(member, cx);

        let (Some(tmdb), false) = (self.tmdb.clone(), missing.is_empty()) else {
            return;
        };
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move { library_sync::cast_photos(&paths, &tmdb, &missing) })
                .await;
            this.update(cx, |view, cx| {
                let Some(pane) = view.cast_pane.as_mut().filter(|p| {
                    p.key.is_none()
                        && p.person
                            .as_ref()
                            .is_some_and(|v| v.member.person_id == person_id)
                }) else {
                    return;
                };
                match fetched {
                    Ok(photos) => pane.photos.extend(
                        photos
                            .into_iter()
                            .filter_map(|(id, path)| Some((id, path.to_str()?.to_string()))),
                    ),
                    Err(e) => tracing::warn!("person photo: {e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Library titles a person appears in, newest first.
    pub(super) fn library_credits(&self, person_id: i64) -> Vec<(LibraryItem, Option<String>)> {
        let found = self.db.titles_with_person(person_id).unwrap_or_else(|e| {
            tracing::error!("reading credits: {e}");
            Vec::new()
        });
        found
            .into_iter()
            .filter_map(|(key, character)| {
                let list = match key.kind {
                    ItemKind::Movie => &self.movies,
                    ItemKind::Show => &self.shows,
                };
                let item = list.iter().find(|i| i.key == key)?.clone();
                Some((item, character))
            })
            .collect()
    }

    fn open_person(&mut self, member: CastMember, cx: &mut Context<Self>) {
        if member.person_id == 0 {
            return;
        }
        let person_id = member.person_id;
        let credits = self.library_credits(person_id);
        let cached = self.known_for.get(&person_id).cloned();
        let known_for = match (&cached, &self.tmdb) {
            (Some(titles), _) => KnownFor::Ready(titles.clone()),
            (None, Some(_)) => KnownFor::Loading,
            (None, None) => KnownFor::Unavailable,
        };
        let Some(pane) = &mut self.cast_pane else {
            return;
        };
        pane.person = Some(PersonView {
            member,
            credits,
            known_for,
        });
        self.person_scroll.reset();
        cx.notify();

        let (None, Some(tmdb)) = (cached, self.tmdb.clone()) else {
            return;
        };
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let fetched =
                cx.background_executor()
                    .spawn(async move {
                        library_sync::known_for(&paths, &tmdb, person_id, KNOWN_FOR_LIMIT)
                    })
                    .await;
            this.update(cx, |view, cx| {
                let result = match fetched {
                    Ok(found) => {
                        let titles: Rc<Vec<KnownTitle>> = Rc::new(
                            found
                                .into_iter()
                                .map(|(credit, thumb)| KnownTitle {
                                    credit,
                                    thumb: thumb.and_then(|p| p.to_str().map(str::to_string)),
                                })
                                .collect(),
                        );
                        view.known_for.insert(person_id, titles.clone());
                        KnownFor::Ready(titles)
                    }
                    Err(e) => {
                        tracing::warn!("known for: {e:#}");
                        KnownFor::Unavailable
                    }
                };
                // Only if they're still looking at the same person.
                if let Some(person) = view
                    .cast_pane
                    .as_mut()
                    .and_then(|p| p.person.as_mut())
                    .filter(|p| p.member.person_id == person_id)
                {
                    person.known_for = result;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether a TMDB title is already in the library (it's then shown under "In your
    /// library" instead).
    fn in_library(&self, kind: ItemKind, tmdb_id: i64) -> bool {
        let list = match kind {
            ItemKind::Movie => &self.movies,
            ItemKind::Show => &self.shows,
        };
        list.iter()
            .any(|i| i.metadata.as_ref().and_then(|m| m.tmdb_id) == Some(tmdb_id))
    }

    /// "Known for": the person's best-known titles you don't have, each opening its TMDB page.
    fn known_for_section(&self, person: &PersonView, cx: &mut Context<Self>) -> Option<AnyElement> {
        let heading = div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::text_secondary())
            .child("Known for");
        let body: AnyElement = match &person.known_for {
            KnownFor::Unavailable => return None,
            KnownFor::Loading => div()
                .text_sm()
                .text_color(theme::text_tertiary())
                .child("Loading…")
                .into_any_element(),
            KnownFor::Ready(titles) => {
                let posters: Vec<AnyElement> = titles
                    .iter()
                    .filter(|t| !self.in_library(t.credit.kind, t.credit.tmdb_id))
                    .enumerate()
                    .map(|(ix, t)| self.known_title_card(ix, t, cx))
                    .collect();
                if posters.is_empty() {
                    return None;
                }
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .children(posters)
                    .into_any_element()
            }
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(heading)
                .child(body)
                .into_any_element(),
        )
    }

    fn known_title_card(&self, ix: usize, t: &KnownTitle, cx: &mut Context<Self>) -> AnyElement {
        let credit = &t.credit;
        let url = format!(
            "https://www.themoviedb.org/{}/{}",
            match credit.kind {
                ItemKind::Movie => "movie",
                ItemKind::Show => "tv",
            },
            credit.tmdb_id
        );
        let detail = match (credit.year, &credit.character) {
            (Some(year), Some(c)) => format!("{year} · {c}"),
            (Some(year), None) => year.to_string(),
            (None, Some(c)) => c.clone(),
            (None, None) => String::new(),
        };
        div()
            .id(("known-for", ix))
            .tab_index(0)
            .focus_visible(theme::focus_ring)
            .w(px(CREDIT_POSTER_W))
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .tooltip(theme::text_tooltip("Open on TMDB"))
            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
            .child(
                div()
                    .w(px(CREDIT_POSTER_W))
                    .h(px(CREDIT_POSTER_W * 1.5))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(theme::placeholder(&credit.title, None))
                    .child(art_layer(t.thumb.clone(), 8., ART_THUMB * 2)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(credit.title.clone()),
                    )
                    .when(!detail.is_empty(), |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme::text_tertiary())
                                .truncate()
                                .child(detail),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_cast_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(pane) = &self.cast_pane else {
            return theme::nothing();
        };
        let body = match &pane.person {
            None => self.cast_list(pane, cx),
            Some(person) => self.person_view(pane, person, cx),
        };
        let header = div()
            .flex_shrink_0()
            .h(px(NAV_HEIGHT + 8.))
            .px_6()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(rgba(0xffffff1f))
            .when(pane.person.is_some() && pane.key.is_some(), |d| {
                d.child(
                    round_button(
                        "cast-back",
                        "icons/chevron-left.svg",
                        32.,
                        cx.listener(|this, _, _, cx| {
                            if let Some(pane) = &mut this.cast_pane {
                                pane.person = None;
                                cx.notify();
                            }
                        }),
                    )
                    .tooltip(theme::text_tooltip("Back to cast")),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_family(theme::FONT_DISPLAY)
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if pane.key.is_some() {
                                "Cast & Crew"
                            } else {
                                "Search"
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_tertiary())
                            .truncate()
                            .child(pane.title.clone()),
                    ),
            )
            .child(
                round_button(
                    "cast-close",
                    "icons/window-close.svg",
                    32.,
                    cx.listener(|this, _, _, cx| this.close_cast(cx)),
                )
                .tooltip(theme::text_tooltip("Close  Esc")),
            );

        let sheet = div()
            .id("cast-sheet")
            .occlude()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(SHEET_W))
            .flex()
            .flex_col()
            // True black with a light edge, like the other panels.
            .bg(rgba(0x000000ff))
            .border_l_1()
            .border_color(rgba(0xffffff2e))
            .shadow(theme::frost_shadow())
            .child(header)
            .child(body)
            .with_animation(
                ("cast-sheet-in", self.cast_epoch),
                Animation::new(theme::motion(260)).with_easing(ease_out_quint()),
                |el, t| el.right(px(-(1. - t) * SHEET_W * 0.35)).opacity(t),
            );

        div()
            .id("cast-overlay")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000099))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_cast(cx)),
            )
            .child(sheet)
            .into_any_element()
    }

    fn avatar(&self, pane: &CastPane, member: &CastMember, size: f32) -> AnyElement {
        avatar_circle(
            &member.name,
            pane.photos.get(&member.person_id).map(String::as_str),
            size,
        )
    }

    fn cast_list(&self, pane: &CastPane, cx: &mut Context<Self>) -> AnyElement {
        let rows = pane.members.iter().enumerate().map(|(ix, member)| {
            let others = pane.elsewhere.get(&member.person_id).copied().unwrap_or(0);
            let can_open = member.person_id != 0;
            let chosen = member.clone();
            div()
                .id(("cast-member", ix))
                .tab_index(0)
                .focus_visible(theme::focus_ring)
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_4()
                .px_3()
                .py_2()
                .rounded(px(10.))
                .when(can_open, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgba(0xffffff0f)))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_person(chosen.clone(), cx)),
                        )
                })
                .child(self.avatar(pane, member, AVATAR))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(member.name.clone()),
                        )
                        .children(member.character.clone().map(|c| {
                            div()
                                .text_xs()
                                .text_color(theme::text_tertiary())
                                .truncate()
                                .child(c)
                        })),
                )
                .when(others > 0, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .px_2()
                            .py_0p5()
                            .rounded_full()
                            .bg(rgba(0xffffff14))
                            .text_xs()
                            .text_color(theme::text_secondary())
                            .child(format!("{others} more")),
                    )
                })
        });
        div()
            .id("cast-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.cast_list_scroll.handle)
            .p_3()
            .flex()
            .flex_col()
            .gap_0p5()
            .children(rows)
            .child(self.cast_list_scroll.scrollbar(cx.entity_id()))
            .into_any_element()
    }

    fn person_view(
        &self,
        pane: &CastPane,
        person: &PersonView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let member = &person.member;
        // "Hanna in Heat"; crew read "Director of Heat".
        let here = member.character.as_ref().map(|c| {
            let of = if matches!(c.as_str(), "Director" | "Creator") {
                "of"
            } else {
                "in"
            };
            format!("{c} {of} {}", pane.title)
        });
        let known_for = self.known_for_section(person, cx);
        let posters = person
            .credits
            .iter()
            .enumerate()
            .map(|(ix, (item, character))| {
                let meta = item.metadata.as_ref();
                let title = item.display_title().to_string();
                let current = pane.key.as_ref() == Some(&item.key);
                let target = item.clone();
                div()
                    .id(("person-credit", ix))
                    .tab_index(0)
                    .focus_visible(theme::focus_ring)
                    .w(px(CREDIT_POSTER_W))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cast_pane = None;
                        if !current {
                            this.open(&target, cx);
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .relative()
                            .w(px(CREDIT_POSTER_W))
                            .h(px(CREDIT_POSTER_W * 1.5))
                            .rounded(px(8.))
                            .overflow_hidden()
                            .bg(theme::placeholder(&title, meta.and_then(|m| m.accent_rgb)))
                            .child(art_layer(
                                meta.and_then(|m| m.poster_file.clone()),
                                8.,
                                ART_THUMB * 2,
                            ))
                            .when(current, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .right_0()
                                        .bottom_0()
                                        .py_1()
                                        .bg(rgba(0x000000cc))
                                        .flex()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(theme::text_secondary())
                                        .child("This title"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(title),
                            )
                            .children(character.clone().map(|c| {
                                div()
                                    .text_xs()
                                    .text_color(theme::text_tertiary())
                                    .truncate()
                                    .child(c)
                            })),
                    )
            });
        let others = person
            .credits
            .iter()
            .filter(|(item, _)| pane.key.as_ref() != Some(&item.key))
            .count();

        div()
            .id("person-view")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.person_scroll.handle)
            .p_6()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(self.avatar(pane, member, PERSON_AVATAR))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_family(theme::FONT_DISPLAY)
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(member.name.clone()),
                            )
                            .children(here.map(|h| {
                                div().text_sm().text_color(theme::text_secondary()).child(h)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text_secondary())
                            .child("In your library"),
                    )
                    .when(others == 0, |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme::text_tertiary())
                                .child(format!(
                                    "Nothing else with {} in your library yet.",
                                    member.name
                                )),
                        )
                    })
                    .child(div().flex().flex_wrap().gap_4().children(posters)),
            )
            .children(known_for)
            .child(self.person_scroll.scrollbar(cx.entity_id()))
            .into_any_element()
    }
}

/// A round photo of someone, or their initials on a colour picked from their name while there
/// is no photo.
pub(super) fn avatar_circle(name: &str, photo: Option<&str>, size: f32) -> AnyElement {
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect();
    div()
        .relative()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .bg(theme::placeholder(name, None))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(size * 0.34))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text_secondary())
        .child(initials)
        .children(photo.map(|photo| {
            div().absolute().inset_0().child(
                crate::image_cache::art_image_square(photo, (size * 2.) as u32)
                    .size_full()
                    .rounded_full()
                    .object_fit(ObjectFit::Cover),
            )
        }))
        .into_any_element()
}
