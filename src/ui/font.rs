use std::sync::atomic::{AtomicU8, Ordering};

use xilem::masonry::core::ArcStr;
use xilem::view::{Label, Prose, TextInput};

use crate::settings::UiFont;

static CURRENT_FONT: AtomicU8 = AtomicU8::new(0);

pub(super) fn set_current(font: UiFont) {
    CURRENT_FONT.store(font_index(font), Ordering::Relaxed);
}

fn font_index(font: UiFont) -> u8 {
    match font {
        UiFont::System => 0,
        UiFont::Sans => 1,
        UiFont::Serif => 2,
        UiFont::Monospace => 3,
        UiFont::Rounded => 4,
    }
}

fn current_stack() -> &'static str {
    match CURRENT_FONT.load(Ordering::Relaxed) {
        1 => "sans-serif, 'Noto Sans CJK JP', system-ui",
        2 => "serif, 'Noto Sans CJK JP', system-ui",
        3 => "monospace, 'Noto Sans CJK JP', system-ui",
        4 => "ui-rounded, 'Noto Sans CJK JP', system-ui",
        _ => "system-ui, 'Noto Sans CJK JP', sans-serif",
    }
}

pub(super) fn label(text: impl Into<ArcStr>) -> Label {
    xilem::view::label(text).font(current_stack())
}

pub(super) fn prose(text: impl Into<ArcStr>) -> Prose {
    xilem::view::prose(text).font(current_stack())
}

pub(super) fn text_input<F, State, Action>(
    contents: String,
    on_changed: F,
) -> TextInput<State, Action>
where
    State: 'static,
    Action: 'static,
    F: Fn(&mut State, String) -> Action + Send + Sync + 'static,
{
    xilem::view::text_input(contents, on_changed).font(current_stack())
}
