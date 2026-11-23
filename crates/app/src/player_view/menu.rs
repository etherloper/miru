//! The player's settings menu, compact like a web player's: a short list (Audio, Subtitles,
//! Chapters, Media info) showing the current choice, each opening its own page with a back
//! arrow. Choosing an option applies it and stays on the page. From the keyboard: Up/Down move,
//! Enter or Right opens or chooses, Left or Backspace goes back, Esc closes.

use gpui::AnyElement;
use miru_player::MediaInfo;

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

/// Which page of the menu is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuPage {
    Main,
    Section(MenuSection),
    Info,
}

/// An entry on the main page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainRow {
    Section(MenuSection),
    Info,
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

const MENU_W: f32 = 300.;
const ROW_H: f32 = 40.;

impl PlayerView {
    fn main_rows(&self) -> Vec<MainRow> {
        let mut rows = vec![
            MainRow::Section(MenuSection::Audio),
            MainRow::Section(MenuSection::Subtitles),
        ];
        if !self.chapters.is_empty() {
            rows.push(MainRow::Section(MenuSection::Chapters));
        }
        rows.push(MainRow::Info);
        rows
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
                    primary: c
                        .title
                        .clone()
                        .unwrap_or_else(|| format!("Chapter {}", ix + 1))
                        .into(),
                    detail: format_time(c.time).into(),
                    selected: self.current_chapter == Some(ix),
                    pick: Pick::Chapter(ix),
                })
                .collect(),
        }
    }

    /// What a section currently has chosen, shown on the right of its row.
    fn menu_summary(&self, section: MenuSection) -> String {
        match section {
            MenuSection::Audio => self
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Audio && t.selected)
                .map_or_else(String::new, |t| t.primary()),
            MenuSection::Subtitles => self
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Subtitle && t.selected)
                .map_or_else(|| "Off".into(), |t| t.primary()),
            MenuSection::Chapters => self.current_chapter.map_or_else(String::new, |ix| {
                format!("{} of {}", ix + 1, self.chapters.len())
            }),
        }
    }

    pub(super) fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.menu = None;
        } else {
            self.tracks = self.session.player.tracks();
            self.audio_output = self.session.player.audio_output_layout();
            self.menu = Some(MenuPage::Main);
            self.menu_cursor = 0;
            self.menu_open_epoch += 1;
        }
        self.poke(cx);
    }

    fn open_page(&mut self, page: MenuPage, cx: &mut Context<Self>) {
        self.menu_cursor = match page {
            MenuPage::Section(section) => self
                .menu_choices(section)
                .iter()
                .position(|c| c.selected)
                .unwrap_or(0),
            MenuPage::Info => {
                self.media_info = Some(self.session.player.media_info());
                0
            }
            MenuPage::Main => 0,
        };
        self.menu = Some(page);
        self.menu_forward = true;
        self.menu_page_epoch += 1;
        self.poke(cx);
    }

    /// Back to the main page, with the row just left highlighted.
    fn menu_back(&mut self, cx: &mut Context<Self>) {
        let from = match self.menu {
            Some(MenuPage::Section(s)) => Some(MainRow::Section(s)),
            Some(MenuPage::Info) => Some(MainRow::Info),
            _ => None,
        };
        self.menu = Some(MenuPage::Main);
        self.menu_forward = false;
        self.menu_page_epoch += 1;
        self.menu_cursor = from
            .and_then(|row| self.main_rows().iter().position(|r| *r == row))
            .unwrap_or(0);
        self.poke(cx);
    }

    fn open_main_row(&mut self, row: MainRow, cx: &mut Context<Self>) {
        match row {
            MainRow::Section(section) => self.open_page(MenuPage::Section(section), cx),
            MainRow::Info => self.open_page(MenuPage::Info, cx),
        }
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
    pub(super) fn menu_key(&mut self, key: &str, _shift: bool, cx: &mut Context<Self>) -> bool {
        let Some(page) = self.menu else {
            return false;
        };
        let count = match page {
            MenuPage::Main => self.main_rows().len(),
            MenuPage::Section(section) => self.menu_choices(section).len(),
            MenuPage::Info => 0,
        };
        match key {
            "escape" => {
                self.menu = None;
                self.poke(cx);
            }
            "left" | "backspace" if page != MenuPage::Main => self.menu_back(cx),
            "down" | "up" if count > 0 => {
                self.menu_cursor = if key == "down" {
                    (self.menu_cursor + 1).min(count - 1)
                } else {
                    self.menu_cursor.saturating_sub(1)
                };
                self.poke(cx);
            }
            "enter" | "space" | "right" => match page {
                MenuPage::Main => {
                    if let Some(row) = self.main_rows().get(self.menu_cursor).copied() {
                        self.open_main_row(row, cx);
                    }
                }
                MenuPage::Section(section) if key != "right" => {
                    if let Some(pick) = self
                        .menu_choices(section)
                        .get(self.menu_cursor)
                        .map(|c| c.pick)
                    {
                        self.apply_pick(pick, cx);
                    }
                }
                _ => {}
            },
            // Swallow the rest (seeking, volume) while the menu has the keyboard.
            "left" | "up" | "down" => {}
            _ => return false,
        }
        true
    }

    pub(super) fn render_player_menu(
        &self,
        page: MenuPage,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let max_list = (f32::from(window.viewport_size().height) - 320.).clamp(160., 360.);
        let body: AnyElement = match page {
            MenuPage::Main => self.render_main_page(cx),
            MenuPage::Section(section) => self.render_section_page(section, max_list, cx),
            MenuPage::Info => self.render_info_page(cx),
        };
        // A page change slides the new page in (from the right going in, the left coming
        // back) as it fades up.
        let shift = if self.menu_forward { 28. } else { -28. };
        let body = div().child(body).with_animation(
            ("menu-page", self.menu_page_epoch),
            Animation::new(theme::motion(240)).with_easing(gpui::ease_out_quint()),
            move |el, t| el.opacity(t).ml(px(shift * (1. - t))),
        );
        div()
            .id("player-menu")
            // Above the click-outside layer: without this, pressing a row also reached that
            // layer, which closed the menu before the click could land.
            .occlude()
            .absolute()
            .right(px(EDGE))
            .bottom(px(150.))
            .w(px(MENU_W))
            .p_1p5()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .overflow_hidden()
            .bg(rgba(0x000000eb))
            .border_1()
            .border_color(rgba(0xffffff33))
            .shadow(theme::card_shadow(1.))
            .text_color(theme::text())
            .child(body)
            // Opening: rises a little and fades in.
            .with_animation(
                ("menu-open", self.menu_open_epoch),
                Animation::new(theme::motion(200)).with_easing(gpui::ease_out_quint()),
                |el, t| el.opacity(t).bottom(px(138. + 12. * t)),
            )
    }

    fn menu_row(id: (&'static str, usize), highlighted: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .h(px(ROW_H))
            .flex_shrink_0()
            .px_3()
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(8.))
            .cursor_pointer()
            .text_sm()
            .when(highlighted, |d| d.bg(rgba(0xffffff1a)))
            .hover(|s| s.bg(rgba(0xffffff1a)))
    }

    fn render_main_page(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .children(self.main_rows().into_iter().enumerate().map(|(ix, row)| {
                let (icon_path, label, value) = match row {
                    MainRow::Section(s) => (s.icon(), s.label(), self.menu_summary(s)),
                    MainRow::Info => ("icons/info.svg", "Media info", String::new()),
                };
                Self::menu_row(("menu-main", ix), ix == self.menu_cursor)
                    .child(icon(icon_path, 17., theme::text()))
                    .child(div().flex_shrink_0().child(label))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_end()
                            .text_color(theme::text_secondary())
                            .child(div().truncate().child(value)),
                    )
                    .child(icon(
                        "icons/chevron-right.svg",
                        14.,
                        theme::text_secondary(),
                    ))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_main_row(row, cx)),
                    )
            }))
            .into_any_element()
    }

    /// The header of a sub-page: a back arrow and its title.
    fn page_header(title: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("menu-back")
                    .h(px(ROW_H))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|s| s.bg(rgba(0xffffff14)))
                    .child(icon("icons/chevron-left.svg", 16., theme::text()))
                    .child(title)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.menu_back(cx))),
            )
            .child(div().mx_1().my_1().h(px(1.)).bg(rgba(0xffffff1f)))
    }

    fn render_section_page(
        &self,
        section: MenuSection,
        max_list: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choices = self.menu_choices(section);
        let empty = choices.is_empty();
        let rows = choices.into_iter().enumerate().map(|(ix, choice)| {
            let Choice {
                primary,
                detail,
                selected,
                pick,
            } = choice;
            Self::menu_row(("menu-choice", ix), ix == self.menu_cursor)
                .text_color(if selected {
                    theme::text()
                } else {
                    theme::text_secondary()
                })
                .child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .children(selected.then(|| icon("icons/check.svg", 15., theme::text()))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(primary),
                )
                .when(!detail.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(110.))
                            .truncate()
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
        let rows: Vec<_> = rows.collect();
        let header = Self::page_header(section.label(), cx);
        let footer = (section == MenuSection::Audio).then(|| {
            let output = match self.audio_output.as_deref() {
                Some("stereo") => "Stereo".to_string(),
                Some("mono") => "Mono".to_string(),
                Some(layout) => format!("{layout} surround"),
                None => "—".to_string(),
            };
            div()
                .mt_1()
                .mx_1()
                .px_2()
                .pt_2()
                .pb_1()
                .border_t_1()
                .border_color(rgba(0xffffff1f))
                .flex()
                .justify_between()
                .text_xs()
                .text_color(theme::text_tertiary())
                .child("Output")
                .child(div().text_color(theme::text_secondary()).child(output))
        });
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id(("menu-list", section as usize))
                    .max_h(px(max_list))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
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
            .children(footer)
            .into_any_element()
    }

    fn render_info_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let info = self.media_info.clone().unwrap_or_default();
        let rows = info_rows(&info).into_iter().map(|(label, value)| {
            div()
                .px_3()
                .py_1p5()
                .flex()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .w(px(76.))
                        .flex_shrink_0()
                        .text_color(theme::text_tertiary())
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(value))
        });
        div()
            .flex()
            .flex_col()
            .child(Self::page_header("Media info", cx))
            .child(div().pb_1().flex().flex_col().children(rows))
            .into_any_element()
    }
}

/// The Media info page's lines, skipping anything mpv didn't report.
fn info_rows(info: &MediaInfo) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let (Some(w), Some(h)) = (info.width, info.height) {
        let class = match (w, h) {
            (w, h) if w >= 3800 || h >= 2100 => " · 4K",
            (w, h) if w >= 1900 || h >= 1060 => " · 1080p",
            (w, h) if w >= 1260 || h >= 700 => " · 720p",
            _ => " · SD",
        };
        rows.push(("Resolution", format!("{w} × {h}{class}")));
    }
    let mut video = Vec::new();
    if let Some(codec) = &info.video_codec {
        video.push(codec_name(codec));
    }
    if let Some(fps) = info.fps {
        video.push(format!("{} fps", trim_float(fps, 3)));
    }
    video.push(
        match info.gamma.as_deref() {
            Some("pq") => "HDR (PQ)",
            Some("hlg") => "HDR (HLG)",
            _ => "SDR",
        }
        .to_string(),
    );
    if info.video_codec.is_some() {
        rows.push(("Video", video.join(" · ")));
    }
    let mut audio = Vec::new();
    if let Some(codec) = &info.audio_codec {
        audio.push(codec_name(codec));
    }
    if let Some(channels) = &info.audio_channels {
        audio.push(match channels.as_str() {
            "stereo" => "Stereo".to_string(),
            "mono" => "Mono".to_string(),
            other => other.to_string(),
        });
    }
    if let Some(rate) = info.sample_rate {
        audio.push(format!("{} kHz", trim_float(rate as f64 / 1000., 1)));
    }
    if !audio.is_empty() {
        rows.push(("Audio", audio.join(" · ")));
    }
    let bitrate = |bps: f64| {
        if bps >= 1_000_000. {
            format!("{:.1} Mb/s", bps / 1_000_000.)
        } else {
            format!("{:.0} kb/s", bps / 1000.)
        }
    };
    let rates: Vec<String> = [
        info.video_bitrate.map(|b| format!("Video {}", bitrate(b))),
        info.audio_bitrate.map(|b| format!("Audio {}", bitrate(b))),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !rates.is_empty() {
        rows.push(("Bitrate", rates.join(" · ")));
    }
    rows.push((
        "Decoding",
        info.hwdec
            .as_ref()
            .map_or_else(|| "Software".to_string(), |h| format!("Hardware ({h})")),
    ));
    if let Some(container) = &info.container {
        let name = match container.as_str() {
            "mkv" | "matroska" | "matroska,webm" => "Matroska (MKV)".to_string(),
            c if c.contains("mp4") => "MP4".to_string(),
            c => c.to_uppercase(),
        };
        rows.push(("Container", name));
    }
    if let Some(bytes) = info.size_bytes {
        let gb = bytes as f64 / 1_073_741_824.;
        rows.push((
            "File size",
            if gb >= 1. {
                format!("{gb:.2} GB")
            } else {
                format!("{:.0} MB", bytes as f64 / 1_048_576.)
            },
        ));
    }
    if let Some(name) = info
        .path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_name())
    {
        rows.push(("File", name.to_string_lossy().into_owned()));
    }
    rows
}

fn codec_name(codec: &str) -> String {
    match codec {
        "hevc" | "h265" => "HEVC",
        "h264" => "H.264",
        "av1" => "AV1",
        "vp9" => "VP9",
        "mpeg2video" => "MPEG-2",
        "eac3" => "E-AC-3",
        "ac3" => "AC-3",
        "truehd" => "Dolby TrueHD",
        "dts" => "DTS",
        "aac" => "AAC",
        "flac" => "FLAC",
        "opus" => "Opus",
        "mp3" => "MP3",
        "vorbis" => "Vorbis",
        other => return other.to_uppercase(),
    }
    .to_string()
}

/// A number with at most `places` decimals and no trailing zeros (23.976, 24, 48).
fn trim_float(value: f64, places: usize) -> String {
    let s = format!("{value:.places$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_a_file() {
        let info = MediaInfo {
            path: Some(r"D:\Films\Heat (1995).mkv".into()),
            container: Some("mkv".into()),
            size_bytes: Some(15_032_385_536),
            video_codec: Some("hevc".into()),
            width: Some(3840),
            height: Some(2160),
            fps: Some(23.976),
            gamma: Some("pq".into()),
            video_bitrate: Some(18_200_000.),
            audio_codec: Some("eac3".into()),
            audio_channels: Some("5.1".into()),
            sample_rate: Some(48_000),
            audio_bitrate: Some(640_000.),
            hwdec: Some("d3d11va".into()),
        };
        let rows = info_rows(&info);
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| *l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Resolution"), Some("3840 × 2160 · 4K"));
        assert_eq!(get("Video"), Some("HEVC · 23.976 fps · HDR (PQ)"));
        assert_eq!(get("Audio"), Some("E-AC-3 · 5.1 · 48 kHz"));
        assert_eq!(get("Bitrate"), Some("Video 18.2 Mb/s · Audio 640 kb/s"));
        assert_eq!(get("File size"), Some("14.00 GB"));
        assert_eq!(get("File"), Some("Heat (1995).mkv"));
    }
}
