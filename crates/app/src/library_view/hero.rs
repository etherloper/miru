//! The home hero as a cached view.
//!
//! The hero is the largest piece of the home page (backdrop crossfade, logo, copy, dots). As
//! its own cached view it is redrawn only when its state changes, not whenever a card is
//! hovered, and its 9-second crossfade no longer rebuilds the shelves below it.

use std::cell::RefCell;

use gpui::{Entity, StyleRefinement, WeakEntity};

use super::*;

pub(super) struct HeroView {
    library: WeakEntity<LibraryView>,
    height: f32,
}

/// Everything the hero shows; it is redrawn when this changes.
#[derive(Clone, PartialEq)]
pub(super) struct HeroState {
    featured: Vec<LibraryItem>,
    index: usize,
    previous: Option<usize>,
    epoch: usize,
    hovering: bool,
    height: f32,
}

/// The hero entity and the state it last drew.
pub(super) struct Hero {
    view: Entity<HeroView>,
    drawn: RefCell<Option<HeroState>>,
}

impl Hero {
    pub fn new(cx: &mut Context<LibraryView>) -> Self {
        let library = cx.weak_entity();
        Self {
            view: cx.new(|_| HeroView {
                library,
                height: 0.,
            }),
            drawn: RefCell::new(None),
        }
    }
}

impl HeroView {
    /// Runs `f` on the library (where the hero's state and actions live).
    pub(super) fn on_library(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut LibraryView, &mut Context<LibraryView>),
    ) {
        self.library.update(cx, f).ok();
    }
}

impl LibraryView {
    /// The hero embedded as a cached view, redrawn only when its state changed.
    pub(super) fn hero_element(&self, height: f32, cx: &mut Context<Self>) -> AnyElement {
        if self.featured.is_empty() {
            return div().h(px(NAV_HEIGHT + 120.)).into_any_element();
        }
        let state = HeroState {
            featured: self.featured.clone(),
            index: self.hero_index,
            previous: self.hero_previous,
            epoch: self.hero_epoch,
            hovering: self.hovered.as_ref().is_some_and(|h| h.starts_with("hero")),
            height,
        };
        let changed = self.hero.drawn.borrow().as_ref() != Some(&state);
        if changed {
            self.hero.view.update(cx, |hero, cx| {
                hero.height = height;
                cx.notify();
            });
            *self.hero.drawn.borrow_mut() = Some(state);
        }
        self.hero
            .view
            .clone()
            .cached(StyleRefinement::default().w_full().h(px(height)))
            .into_any_element()
    }
}

impl Render for HeroView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(library) = self.library.upgrade() else {
            return div().into_any_element();
        };
        library.read(cx).hero_contents(self.height, cx)
    }
}
