//! The player's one menu for audio, subtitles and chapters: sections on the left (each showing
//! what's chosen), that section's options on the right. Choosing an option applies it and
//! keeps the menu open. From the keyboard: Tab or Left/Right switch section, Up/Down move,
//! Enter chooses, Esc closes.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuSection {
    Audio,
    Subtitles,
    Chapters,
}

impl MenuSection {
    fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Subtitles => "Subtitles",
            Self::Chapters => "Chapters",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Audio => "icons/waveform.svg",
            Self::Subtitles => "icons/captions.svg",
            Self::Chapters => "icons/chapters.svg",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Pick {
    Audio(i64),
    Subtitle(Option<i64>),
    Chapter(usize),
}

struct Choice {
    primary: SharedString,
    detail: SharedString,
    selected: bool,
    pick: Pick,
}

const MENU_W: f32 = 540.;
const RAIL_W: f32 = 184.;

impl PlayerView {
    fn menu_sections(&self) -> Vec<MenuSection> {
        let mut sections = vec![MenuSection::Audio, MenuSection::Subtitles];
        if !self.chapters.is_empty() {
            sections.push(MenuSection::Chapters);
        }
        sections
    }

    fn menu_choices(&self, section: MenuSection) -> Vec<Choice> {
        let tracks = |kind: TrackKind| self.tracks.iter().filter(move |t| t.kind == kind);
        match section {
            MenuSection::Audio => tracks(TrackKind::Audio)
                .map(|t| Choice {
                    primary: t.primary().into(),
                    detail: t.detail().into(),
                    selected: t.selected,
                    pick: Pick::Audio(t.id),
                })
                .collect(),
            MenuSection::Subtitles => {
                let any_on = tracks(TrackKind::Subtitle).any(|t| t.selected);
                std::iter::once(Choice {
                    primary: "Off".into(),
                    detail: SharedString::default(),
                    selected: !any_on,
                    pick: Pick::Subtitle(None),
                })
                .chain(tracks(TrackKind::Subtitle).map(|t| Choice {
                    primary: t.primary().into(),
                    detail: t.detail().into(),
                    selected: t.selected,
                    pick: Pick::Subtitle(Some(t.id)),
                }))
                .collect()
            }
            MenuSection::Chapters => self
                .chapters
                .iter()
                .enumerate()
                .map(|(ix, c)| Choice {
                    primary: format!(
                        "{}  {}",
                        ix + 1,
                        c.title
                            .clone()
                            .unwrap_or_else(|| format!("Chapter {}", ix + 1))
                    )
                    .into(),
                    detail: format_time(c.time).into(),
                    selected: self.current_chapter == Some(ix),
                    pick: Pick::Chapter(ix),
                })
                .collect(),
        }
    }

    /// What a section currently has chosen, shown under its name.
    fn menu_summary(&self, section: MenuSection) -> String {
        match section {
            MenuSection::Audio => self
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Audio && t.selected)
                .map_or_else(|| "None".into(), |t| t.primary()),
            MenuSection::Subtitles => self
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Subtitle && t.selected)
                .map_or_else(|| "Off".into(), |t| t.primary()),
            MenuSection::Chapters => self
                .current_chapter
                .and_then(|ix| self.chapters.get(ix).map(|c| (ix, c)))
                .map_or_else(String::new, |(ix, c)| {
                    let title = c
                        .title
                        .clone()
                        .unwrap_or_else(|| format!("Chapter {}", ix + 1));
                    format!("{} · {title}", ix + 1)
                }),
        }
    }

    pub(super) fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.menu = None;
            self.poke(cx);
        } else {
            self.tracks = self.session.player.tracks();
            self.audio_output = self.session.player.audio_output_layout();
            let section = if self.menu_sections().contains(&self.menu_last) {
                self.menu_last
            } else {
                MenuSection::Audio
            };
            self.open_section(section, cx);
        }
    }

    fn open_section(&mut self, section: MenuSection, cx: &mut Context<Self>) {
        self.menu = Some(section);
        self.menu_last = section;
        self.menu_cursor = self
            .menu_choices(section)
            .iter()
            .position(|c| c.selected)
            .unwrap_or(0);
        self.poke(cx);
    }

    fn apply_pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        match pick {
            Pick::Audio(id) => self.choose_track(TrackKind::Audio, Some(id), cx),
            Pick::Subtitle(id) => self.choose_track(TrackKind::Subtitle, id, cx),
            Pick::Chapter(ix) => self.go_to_chapter(ix, cx),
        }
        self.poke(cx);
    }

    /// Keys while the menu is open. Returns whether the key was used.
    pub(super) fn menu_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> bool {
        let Some(section) = self.menu else {
            return false;
        };
        let sections = self.menu_sections();
        let at = sections.iter().position(|s| *s == section).unwrap_or(0);
        let step_section = |by: isize| {
            let n = sections.len() as isize;
            sections[((at as isize + by).rem_euclid(n)) as usize]
        };
        match key {
            "tab" => self.open_section(step_section(if shift { -1 } else { 1 }), cx),
            "right" => self.open_section(step_section(1), cx),
            "left" => self.open_section(step_section(-1), cx),
            "down" | "up" => {
                let count = self.menu_choices(section).len();
                if count > 0 {
                    self.menu_cursor = if key == "down" {
                        (self.menu_cursor + 1).min(count - 1)
                    } else {
                        self.menu_cursor.saturating_sub(1)
                    };
                    self.poke(cx);
                }
            }
            "enter" | "space" => {
                if let Some(pick) = self
                    .menu_choices(section)
                    .get(self.menu_cursor)
                    .map(|c| c.pick)
                {
                    self.apply_pick(pick, cx);
                }
            }
            "escape" => {
                self.menu = None;
                self.poke(cx);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn render_player_menu(
        &self,
        section: MenuSection,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let height = (f32::from(window.viewport_size().height) - 300.).clamp(220., 420.);

        let rail = div()
            .w(px(RAIL_W))
            .flex_shrink_0()
            .p_2()
            .flex()
            .flex_col()
            .gap_0p5()
            .border_r_1()
            .border_color(rgba(0xffffff1f))
            .children(self.menu_sections().into_iter().map(|s| {
                let on = s == section;
                let summary = self.menu_summary(s);
                div()
                    .id(("menu-section", s as usize))
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_color(if on {
                        theme::text()
                    } else {
                        theme::text_secondary()
                    })
                    .when(on, |d| d.bg(rgba(0xffffff1a)))
                    .when(!on, |d| {
                        d.hover(|st| st.bg(rgba(0xffffff0f)).text_color(theme::text()))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(icon(s.icon(), 15., theme::text_secondary()))
                            .child(s.label()),
                    )
                    .when(!summary.is_empty(), |d| {
                        d.child(
                            div()
                                .pl(px(23.))
                                .text_xs()
                                .text_color(theme::text_tertiary())
                                .truncate()
                                .child(summary),
                        )
                    })
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_section(s, cx)),
                    )
            }));

        let choices = self.menu_choices(section);
        let empty = choices.is_empty();
        let rows = choices.into_iter().enumerate().map(|(ix, choice)| {
            let Choice {
                primary,
                detail,
                selected,
                pick,
            } = choice;
            let cursor = ix == self.menu_cursor;
            div()
                .id(("menu-choice", ix))
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(px(8.))
                .cursor_pointer()
                .text_color(if selected {
                    theme::text()
                } else {
                    theme::text_secondary()
                })
                // The keyboard highlight; the pointer gets the same look on hover.
                .when(cursor, |d| d.bg(rgba(0xffffff14)))
                .hover(|st| st.bg(rgba(0xffffff14)).text_color(theme::text()))
                .child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .children(selected.then(|| icon("icons/check.svg", 15., theme::accent()))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .font_weight(if selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .child(primary),
                        )
                        .when(
                            !detail.is_empty() && !matches!(pick, Pick::Chapter(_)),
                            |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .truncate()
                                        .text_color(theme::text_tertiary())
                                        .child(detail.clone()),
                                )
                            },
                        ),
                )
                .when(matches!(pick, Pick::Chapter(_)), |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::text_tertiary())
                            .child(detail),
                    )
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.menu_cursor = ix;
                    this.apply_pick(pick, cx);
                }))
        });

        let footer = (section == MenuSection::Audio).then(|| {
            let output = match self.audio_output.as_deref() {
                Some("stereo") => "Stereo".to_string(),
                Some("mono") => "Mono".to_string(),
                Some(layout) => format!("{layout} surround"),
                None => "—".to_string(),
            };
            div()
                .flex_shrink_0()
                .mx_2()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(rgba(0xffffff1f))
                .flex()
                .justify_between()
                .text_xs()
                .text_color(theme::text_tertiary())
                .child("Output")
                .child(div().text_color(theme::text_secondary()).child(output))
        });

        let list = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("menu-list", section as usize))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .when(empty, |d| {
                        d.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_sm()
                                .text_color(theme::text_secondary())
                                .child("No tracks"),
                        )
                    })
                    .children(rows),
            )
            .children(footer);

        div()
            .id("player-menu")
            // Above the click-outside layer: without this, pressing a row also reached that
            // layer, which closed the menu before the click could land.
            .occlude()
            .absolute()
            .right(px(EDGE))
            .bottom(px(150.))
            .w(px(MENU_W))
            .h(px(height))
            .flex()
            .rounded(px(14.))
            .overflow_hidden()
            .bg(rgba(0x000000f5))
            .border_1()
            .border_color(rgba(0xffffff38))
            .shadow(theme::card_shadow(1.))
            .child(rail)
            .child(list)
    }
}
