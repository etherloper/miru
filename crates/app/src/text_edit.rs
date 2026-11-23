//! Single-line text editing for the app's fields (search, Fix match, the TMDB key, collection
//! names): a caret and selection that follow the keyboard (arrows, Home/End, Ctrl for whole
//! words, Shift to select, Ctrl+A/C/X/V) and the mouse (click, drag, double-click for a word,
//! triple-click for everything).
//!
//! `TextEdit` holds the text and selection inside whichever view owns the field; that view
//! forwards keys and mouse events to it. `TextEdit::element` draws it and records the shaped
//! line, which the mouse handlers use to turn a pointer position into a place in the text.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, Element, ElementId, FocusHandle,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, Keystroke, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Rgba, ShapedLine, SharedString,
    Stateful, Style, TextRun, Window, fill, point, px, relative, size,
};

/// Caret blink: shown this long after any edit or move, then on and off at this rate.
const BLINK: Duration = Duration::from_millis(530);

/// What a key did to the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The text changed.
    Changed,
    /// Only the caret or selection moved (or text was copied).
    Moved,
    /// Not an editing key (Enter, Escape, Up, Down…): the owner handles it.
    Unhandled,
}

/// Where the field was last drawn, for mapping the pointer to the text.
#[derive(Default)]
struct Drawn {
    line: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    /// Horizontal scroll, so the caret stays visible in text wider than the field.
    scroll: f32,
    /// A repaint is already scheduled for the next blink.
    blink_pending: bool,
}

pub struct TextEdit {
    text: String,
    selection: Range<usize>,
    /// The caret is at the start of the selection (it was made leftwards).
    reversed: bool,
    selecting: bool,
    /// Masks the text (secrets such as API keys).
    masked: bool,
    /// Characters the field accepts; others are dropped from typing and paste.
    accepts: fn(char) -> bool,
    changed_at: Rc<Cell<Instant>>,
    drawn: Rc<RefCell<Drawn>>,
}

impl TextEdit {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            selection: end..end,
            reversed: false,
            selecting: false,
            masked: false,
            accepts: |c| !c.is_control(),
            changed_at: Rc::new(Cell::new(Instant::now())),
            drawn: Rc::default(),
        }
    }

    /// A masked field that accepts no whitespace (keys and tokens).
    pub fn secret() -> Self {
        Self {
            masked: true,
            accepts: |c| !c.is_whitespace() && !c.is_control(),
            ..Self::new("")
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text, with the caret at the end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.move_to(self.text.len());
    }

    pub fn clear(&mut self) {
        self.set_text("");
    }

    pub fn select_all(&mut self) {
        self.selection = 0..self.text.len();
        self.reversed = false;
        self.touch();
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn touch(&self) {
        self.changed_at.set(Instant::now());
    }

    fn move_to(&mut self, offset: usize) {
        self.selection = offset..offset;
        self.reversed = false;
        self.touch();
    }

    fn select_to(&mut self, offset: usize) {
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        self.touch();
    }

    fn replace_selection(&mut self, new: &str) {
        let clean: String = new.chars().filter(|c| (self.accepts)(*c)).collect();
        let range = self.selection.clone();
        self.text.replace_range(range.clone(), &clean);
        self.move_to(range.start + clean.len());
    }

    fn previous_char(&self, offset: usize) -> usize {
        self.text[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_char(&self, offset: usize) -> usize {
        self.text[offset..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| offset + c.len_utf8())
    }

    /// Start of the word before `offset` (skipping spaces first), as Ctrl+Left moves.
    fn previous_word(&self, offset: usize) -> usize {
        let before: Vec<(usize, char)> = self.text[..offset].char_indices().collect();
        let mut i = before.len();
        while i > 0 && !is_word(before[i - 1].1) {
            i -= 1;
        }
        while i > 0 && is_word(before[i - 1].1) {
            i -= 1;
        }
        before.get(i).map_or(offset, |(ix, _)| *ix).min(offset)
    }

    /// End of the word after `offset`, as Ctrl+Right moves.
    fn next_word(&self, offset: usize) -> usize {
        let mut chars = self.text[offset..].char_indices().peekable();
        while chars.next_if(|(_, c)| !is_word(*c)).is_some() {}
        while chars.next_if(|(_, c)| is_word(*c)).is_some() {}
        chars.peek().map_or(self.text.len(), |(i, _)| offset + i)
    }

    /// The word (or run of spaces/punctuation) around `offset`, for double-click.
    fn word_at(&self, offset: usize) -> Range<usize> {
        let kind = self.text[offset..]
            .chars()
            .next()
            .or_else(|| self.text[..offset].chars().next_back())
            .map(is_word);
        let Some(kind) = kind else {
            return offset..offset;
        };
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word(*c) == kind)
            .last()
            .map_or(offset, |(i, _)| i);
        let end = self.text[offset..]
            .char_indices()
            .find(|(_, c)| is_word(*c) != kind)
            .map_or(self.text.len(), |(i, _)| offset + i);
        start..end
    }

    /// Applies an editing key. Returns `Unhandled` for keys the field doesn't use.
    pub fn handle_key(&mut self, k: &Keystroke, cx: &mut App) -> KeyOutcome {
        let command = k.modifiers.control || k.modifiers.platform;
        let shift = k.modifiers.shift;
        let before = self.text.clone();
        let cursor = self.cursor();
        match k.key.as_str() {
            "left" | "right" | "home" | "end" => {
                let target = match k.key.as_str() {
                    "left" if command => self.previous_word(cursor),
                    "left" if !shift && !self.selection.is_empty() => self.selection.start,
                    "left" => self.previous_char(cursor),
                    "right" if command => self.next_word(cursor),
                    "right" if !shift && !self.selection.is_empty() => self.selection.end,
                    "right" => self.next_char(cursor),
                    "home" => 0,
                    _ => self.text.len(),
                };
                if shift {
                    self.select_to(target);
                } else {
                    self.move_to(target);
                }
            }
            "backspace" | "delete" => {
                if self.selection.is_empty() {
                    let target = match (k.key.as_str(), command) {
                        ("backspace", true) => self.previous_word(cursor),
                        ("backspace", false) => self.previous_char(cursor),
                        (_, true) => self.next_word(cursor),
                        (_, false) => self.next_char(cursor),
                    };
                    self.selection = target.min(cursor)..target.max(cursor);
                }
                self.replace_selection("");
            }
            "a" if command => self.select_all(),
            "c" | "x" if command => {
                // A masked field never puts its secret on the clipboard.
                if !self.selection.is_empty() && !self.masked {
                    let copied = self.text[self.selection.clone()].to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(copied));
                    if k.key == "x" {
                        self.replace_selection("");
                    }
                }
            }
            "v" if command => {
                if let Some(pasted) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    // One line: line breaks and tabs become spaces.
                    let line: String = pasted
                        .trim()
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .collect();
                    self.replace_selection(&line);
                }
            }
            _ if command || k.modifiers.alt => return KeyOutcome::Unhandled,
            _ => match &k.key_char {
                Some(typed) if !typed.chars().any(char::is_control) => {
                    self.replace_selection(typed);
                }
                _ => return KeyOutcome::Unhandled,
            },
        }
        if self.text != before {
            KeyOutcome::Changed
        } else {
            KeyOutcome::Moved
        }
    }

    /// Text offset under a window position, from where the field was last drawn.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let drawn = self.drawn.borrow();
        let (Some(line), Some(bounds)) = (&drawn.line, drawn.bounds) else {
            return self.text.len();
        };
        let x = position.x - bounds.left() + px(drawn.scroll);
        let shown = line.closest_index_for_x(x);
        if self.masked {
            let chars = shown / MASK.len_utf8();
            self.text
                .char_indices()
                .nth(chars)
                .map_or(self.text.len(), |(i, _)| i)
        } else {
            shown.min(self.text.len())
        }
    }

    pub fn mouse_down(&mut self, event: &MouseDownEvent) {
        let offset = self.offset_at(event.position);
        match event.click_count {
            2 => {
                self.selection = self.word_at(offset);
                self.reversed = false;
                self.touch();
            }
            n if n >= 3 => self.select_all(),
            _ if event.modifiers.shift => self.select_to(offset),
            _ => self.move_to(offset),
        }
        self.selecting = event.click_count == 1;
    }

    /// Extends the selection while the button is held. Returns whether anything changed.
    pub fn mouse_move(&mut self, event: &MouseMoveEvent) -> bool {
        if !self.selecting {
            return false;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return false;
        }
        let offset = self.offset_at(event.position);
        if offset == self.cursor() {
            return false;
        }
        self.select_to(offset);
        true
    }

    pub fn mouse_up(&mut self) {
        self.selecting = false;
    }

    /// The field's text line: selection, text (or placeholder) and blinking caret.
    pub fn element(
        &self,
        placeholder: impl Into<SharedString>,
        color: Rgba,
        placeholder_color: Rgba,
        focused: bool,
    ) -> TextLine {
        TextLine {
            text: if self.masked {
                MASK.to_string().repeat(self.text.chars().count())
            } else {
                self.text.clone()
            },
            selection: self.shown_range(self.selection.clone()),
            cursor: self.shown_offset(self.cursor()),
            placeholder: placeholder.into(),
            color,
            placeholder_color,
            focused,
            changed_at: self.changed_at.clone(),
            drawn: self.drawn.clone(),
        }
    }

    fn shown_offset(&self, offset: usize) -> usize {
        if self.masked {
            self.text[..offset].chars().count() * MASK.len_utf8()
        } else {
            offset
        }
    }

    fn shown_range(&self, range: Range<usize>) -> Range<usize> {
        self.shown_offset(range.start)..self.shown_offset(range.end)
    }
}

const MASK: char = '•';

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// Wires a field container to a `TextEdit` the view owns: focus on press, then click and
/// drag to place the caret and select. `edit` finds the field in the view (it may be gone).
pub fn mouse_handlers<V: 'static>(
    el: Stateful<gpui::Div>,
    focus: &FocusHandle,
    edit: fn(&mut V) -> Option<&mut TextEdit>,
    cx: &mut Context<V>,
) -> Stateful<gpui::Div> {
    let focus = focus.clone();
    el.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |view, event: &MouseDownEvent, window, cx| {
            window.focus(&focus, cx);
            if let Some(edit) = edit(view) {
                edit.mouse_down(event);
                cx.notify();
            }
            cx.stop_propagation();
        }),
    )
    .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
        if let Some(edit) = edit(view)
            && edit.mouse_move(event)
        {
            cx.notify();
        }
    }))
    .on_mouse_up(
        MouseButton::Left,
        cx.listener(move |view, _, _, _| {
            if let Some(edit) = edit(view) {
                edit.mouse_up();
            }
        }),
    )
    .on_mouse_up_out(
        MouseButton::Left,
        cx.listener(move |view, _, _, _| {
            if let Some(edit) = edit(view) {
                edit.mouse_up();
            }
        }),
    )
}

/// Draws one line of a `TextEdit`. Built fresh each frame from a snapshot of the field.
pub struct TextLine {
    text: String,
    selection: Range<usize>,
    cursor: usize,
    placeholder: SharedString,
    color: Rgba,
    placeholder_color: Rgba,
    focused: bool,
    changed_at: Rc<Cell<Instant>>,
    drawn: Rc<RefCell<Drawn>>,
}

pub struct TextLinePrepaint {
    line: ShapedLine,
    scroll: f32,
}

impl IntoElement for TextLine {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextLine {
    type RequestLayoutState = ();
    type PrepaintState = TextLinePrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> TextLinePrepaint {
        let style = window.text_style();
        let (shown, color): (SharedString, Rgba) = if self.text.is_empty() {
            (self.placeholder.clone(), self.placeholder_color)
        } else {
            (self.text.clone().into(), self.color)
        };
        let run = TextRun {
            len: shown.len(),
            font: style.font(),
            color: color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(shown, font_size, &[run], None);

        // Keep the caret in view: scroll just enough when it would leave either edge.
        let width = f32::from(bounds.size.width);
        let mut scroll = self.drawn.borrow().scroll;
        if self.text.is_empty() {
            scroll = 0.;
        } else {
            let caret = f32::from(line.x_for_index(self.cursor));
            let content = f32::from(line.width) + 2.;
            if caret - scroll > width - 2. {
                scroll = caret - width + 2.;
            } else if caret < scroll {
                scroll = caret;
            }
            scroll = scroll.clamp(0., (content - width).max(0.));
        }
        TextLinePrepaint { line, scroll }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut TextLinePrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line = &prepaint.line;
        let scroll = px(prepaint.scroll);
        let has_text = !self.text.is_empty();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if has_text && !self.selection.is_empty() {
                let start = bounds.left() + line.x_for_index(self.selection.start) - scroll;
                let end = bounds.left() + line.x_for_index(self.selection.end) - scroll;
                window.paint_quad(fill(
                    Bounds::from_corners(point(start, bounds.top()), point(end, bounds.bottom())),
                    gpui::rgba(0xffffff38),
                ));
            }
            let origin = point(bounds.left() - scroll, bounds.top());
            if let Err(e) = line.paint(
                origin,
                window.line_height(),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            ) {
                tracing::warn!("cannot draw text field: {e}");
            }
            if self.focused && self.selection.is_empty() {
                let since = self.changed_at.get().elapsed();
                let visible =
                    since < BLINK || (since.as_millis() / BLINK.as_millis()).is_multiple_of(2);
                if visible {
                    let x = if has_text {
                        bounds.left() + line.x_for_index(self.cursor) - scroll
                    } else {
                        bounds.left()
                    };
                    let height = (bounds.size.height * 0.8).min(px(20.));
                    let top = bounds.top() + (bounds.size.height - height) / 2.;
                    window.paint_quad(fill(
                        Bounds::new(point(x, top), size(px(2.), height)),
                        self.color,
                    ));
                }
            }
        });

        {
            let mut drawn = self.drawn.borrow_mut();
            drawn.line = Some(prepaint.line.clone());
            drawn.bounds = Some(bounds);
            drawn.scroll = prepaint.scroll;
        }

        // Blink by repainting at the next phase change (rather than every frame).
        if self.focused && !self.drawn.borrow().blink_pending {
            let since = self.changed_at.get().elapsed().as_millis();
            let period = BLINK.as_millis();
            let wait = period - since % period;
            self.drawn.borrow_mut().blink_pending = true;
            let drawn = self.drawn.clone();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(wait as u64 + 5))
                        .await;
                    drawn.borrow_mut().blink_pending = false;
                    cx.update(|window, _| window.refresh()).ok();
                })
                .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_chars() {
        let edit = TextEdit::new("the café's  door");
        assert_eq!(edit.previous_word(edit.text.len()), 13);
        assert_eq!(edit.previous_word(13), 4);
        assert_eq!(edit.next_word(0), 3);
        assert_eq!(edit.next_word(3), 11);
        let e = edit.text.find('é').unwrap_or(0);
        assert_eq!(edit.next_char(e), e + 'é'.len_utf8());
        assert_eq!(edit.previous_char(e + 'é'.len_utf8()), e);
        assert_eq!(edit.word_at(5), 4..11);
        assert_eq!(edit.word_at(0), 0..3);
    }

    #[test]
    fn replacing_a_selection() {
        let mut edit = TextEdit::new("hello world");
        edit.selection = 0..5;
        edit.replace_selection("bye");
        assert_eq!(edit.text(), "bye world");
        assert_eq!(edit.selection, 3..3);
        let mut secret = TextEdit::secret();
        secret.replace_selection(" ab c\n");
        assert_eq!(secret.text(), "abc");
    }
}
