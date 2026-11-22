//! Embedded UI assets. Icons are drawn as alpha masks and tinted with the element's text colour,
//! so their own fill/stroke colour does not matter.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

pub struct Assets;

macro_rules! icons {
    ($($name:literal => $svg:expr),* $(,)?) => {
        const ICONS: &[(&str, &str)] = &[$(($name, $svg)),*];
    };
}

icons! {
    "icons/play.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M8 5.6v12.8a1 1 0 0 0 1.53.85l10.2-6.4a1 1 0 0 0 0-1.7L9.53 4.75A1 1 0 0 0 8 5.6z" fill="black"/></svg>"#,
    "icons/pause.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="6" y="5" width="4" height="14" rx="1.2" fill="black"/><rect x="14" y="5" width="4" height="14" rx="1.2" fill="black"/></svg>"#,
    "icons/chevron-left.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M15 5l-7 7 7 7" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/chevron-right.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9 5l7 7-7 7" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/chevron-down.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 9l6 6 6-6" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/more.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="5.5" cy="12" r="1.9" fill="black"/><circle cx="12" cy="12" r="1.9" fill="black"/><circle cx="18.5" cy="12" r="1.9" fill="black"/></svg>"#,
    "icons/pencil.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4.5 19.5l1-4.2L15.8 5a2 2 0 0 1 2.8 0l.4.4a2 2 0 0 1 0 2.8L8.7 18.5l-4.2 1z" fill="none" stroke="black" stroke-width="2" stroke-linejoin="round"/></svg>"#,
    "icons/circle.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" fill="none" stroke="black" stroke-width="2"/></svg>"#,
    "icons/search.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="black" stroke-width="2.2"/><path d="M15.5 15.5L20 20" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round"/></svg>"#,
    "icons/skip-back.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/><path d="M4.5 3.5v4.2h4.2" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/skip-forward.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/><path d="M19.5 3.5v4.2h-4.2" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/audio.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5H4z" fill="black"/><path d="M15.5 9a4.2 4.2 0 0 1 0 6M18 6.5a7.8 7.8 0 0 1 0 11" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/></svg>"#,
    "icons/waveform.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 10.5v3M8 7v10M12 4v16M16 8v8M20 10.5v3" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round"/></svg>"#,
    "icons/volume-mute.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5H4z" fill="black"/><path d="M16 9.5l5 5M21 9.5l-5 5" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/></svg>"#,
    "icons/chapters.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9 6.5h11M9 12h11M9 17.5h11" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/><circle cx="4.75" cy="6.5" r="1.5" fill="black"/><circle cx="4.75" cy="12" r="1.5" fill="black"/><circle cx="4.75" cy="17.5" r="1.5" fill="black"/></svg>"#,
    "icons/captions.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="14" rx="3" fill="none" stroke="black" stroke-width="2"/><path d="M7 12.5h4M13 12.5h4M7 15.5h7" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/></svg>"#,
    "icons/settings.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="black"><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(45 12 12)"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(90 12 12)"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(135 12 12)"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(180 12 12)"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(225 12 12)"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(270 12 12)"/><rect x="10.6" y="1.8" width="2.8" height="4.6" rx="1" transform="rotate(315 12 12)"/></g><circle cx="12" cy="12" r="6.2" fill="none" stroke="black" stroke-width="3"/><circle cx="12" cy="12" r="2.2" fill="black"/></svg>"#,
    "icons/folder.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3.5 7.5a2 2 0 0 1 2-2h3.8l2 2.2h7.2a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"/></svg>"#,
    "icons/sidebar.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="3.5" y="4.5" width="17" height="15" rx="3" fill="none" stroke="black" stroke-width="1.9"/><path d="M9.5 4.5v15" fill="none" stroke="black" stroke-width="1.9"/></svg>"#,
    "icons/home.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 11l8-6.5 8 6.5V19a1.5 1.5 0 0 1-1.5 1.5H14v-5.5h-4v5.5H5.5A1.5 1.5 0 0 1 4 19z" fill="none" stroke="black" stroke-width="1.9" stroke-linejoin="round"/></svg>"#,
    "icons/film.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="3.5" y="4.5" width="17" height="15" rx="2.5" fill="none" stroke="black" stroke-width="1.9"/><path d="M7.5 4.5v15M16.5 4.5v15M3.5 9.5h4M3.5 14.5h4M16.5 9.5h4M16.5 14.5h4" fill="none" stroke="black" stroke-width="1.9"/></svg>"#,
    "icons/tv.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="3" y="4.5" width="18" height="12.5" rx="2.5" fill="none" stroke="black" stroke-width="1.9"/><path d="M8.5 20.5h7M12 17v3.5" fill="none" stroke="black" stroke-width="1.9" stroke-linecap="round"/></svg>"#,
    "icons/plus.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/></svg>"#,
    "icons/info.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5" fill="none" stroke="black" stroke-width="2"/><path d="M12 11v5.5" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/><circle cx="12" cy="7.8" r="1.2" fill="black"/></svg>"#,
    "icons/check.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="black" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/restart.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12a7 7 0 1 0 2-4.9" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"/><path d="M5 4v4h4" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    "icons/window-min.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M2 6h8" fill="none" stroke="black" stroke-width="1.1"/></svg>"#,
    "icons/window-max.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><rect x="2.5" y="2.5" width="7" height="7" rx="1" fill="none" stroke="black" stroke-width="1.1"/></svg>"#,
    "icons/window-close.svg" => r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M2.5 2.5l7 7M9.5 2.5l-7 7" fill="none" stroke="black" stroke-width="1.1"/></svg>"#,
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, svg)| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_well_formed_svg() {
        for (name, svg) in ICONS {
            assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{name}");
            assert!(Assets.load(name).unwrap().is_some(), "{name}");
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
    }
}
