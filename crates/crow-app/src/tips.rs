//! One-time contextual tips: the app-side global that owns `tips.json` and the
//! rules, plus the card the window draws. `crow_core::tips` holds the state
//! machine; this module decides when the app may ask and how a tip looks.

use std::path::PathBuf;
use std::time::Instant;

use crow_core::{HintsMode, TIPS_FILE, TipBook, TipId, TipState};
use gpui::{App, AppContext, EntityId, Global, IntoElement, Role, SharedString, Task, Window, div, prelude::*, px};

use crate::actions::{ShowCommandPalette, ShowSearch, StartFilter};
use crate::chrome::{Keycap, KeycapWeight};
use crate::keys::{self, KeyScope};
use crate::style::color;
use crate::ui::Ui;

pub struct Tips {
    book: TipBook,
    file: PathBuf,
    /// The window showing the tip, if any.
    owner: Option<EntityId>,
    /// The pending save; a new one waits for it so the file never goes backwards.
    save: Option<Task<()>>,
}

impl Global for Tips {}

/// Load `tips.json` from the state dir. A startup read, like Recent's.
pub fn init(cx: &mut App) {
    let file = cx.global::<Ui>().state_dir.join(TIPS_FILE);
    let book = TipBook::new(TipState::load(&file));
    cx.set_global(Tips { book, file, owner: None, save: None });
}

/// The window showing a tip, if any.
pub fn owner(cx: &App) -> Option<EntityId> {
    cx.global::<Tips>().owner
}

/// Whether `id` is worth a trigger check: hints on, not retired, nothing on
/// screen and its key never used. The cheap test the scroll listener runs per
/// event, so a spent tip costs no event.
pub fn wants(id: TipId, cx: &App) -> bool {
    let tips = cx.global::<Tips>();
    cx.global::<Ui>().settings.hints != HintsMode::Off
        && !tips.book.state().is_retired(id)
        && !tips.book.is_on_screen()
        && uses_of(id, cx) == 0
}

/// Ask to show `id` in `owner`. Checks the hints setting, the key-use count,
/// retirement, one-on-screen and the cooldown; the caller has already checked
/// its own blocking states.
pub fn request(id: TipId, owner: EntityId, cx: &mut App) -> bool {
    let hints_off = cx.global::<Ui>().settings.hints == HintsMode::Off;
    let uses = uses_of(id, cx);
    let now = Instant::now();
    {
        let tips = cx.global_mut::<Tips>();
        if !tips.book.may_show(id, now, uses, hints_off) {
            return false;
        }
        tips.book.show(id, now);
        tips.owner = Some(owner);
    }
    save_in_background(cx);
    true
}

/// `owner` closed its tip (Got it, folder change, another overlay).
pub fn closed(owner: EntityId, cx: &mut App) {
    let tips = cx.global_mut::<Tips>();
    if tips.owner == Some(owner) {
        tips.owner = None;
        tips.book.closed();
    }
}

/// The tip's action ran (key, menu or palette): retire it, and close it when it
/// is the one on screen.
pub fn key_used(id: TipId, cx: &mut App) {
    let (changed, close) = {
        let tips = cx.global_mut::<Tips>();
        let changed = tips.book.retire(id);
        let close = tips.book.on_screen() == Some(id);
        if close {
            tips.book.closed();
            tips.owner = None;
        }
        (changed, close)
    };
    if changed {
        save_in_background(cx);
    }
    if close {
        cx.refresh_windows();
    }
}

/// The shortcut to show for `id`'s action in the context where the tip fires.
pub fn key_text(id: TipId, cx: &App) -> Option<SharedString> {
    match id {
        TipId::Filter => keys::text(&StartFilter, KeyScope::List, cx),
        TipId::Search => keys::text(&ShowSearch, KeyScope::Window, cx),
        TipId::GoTo => keys::text(&ShowCommandPalette, KeyScope::Window, cx),
    }
}

/// The tip's one sentence, with the key as plain text inside it.
pub fn text(id: TipId, key: &str) -> String {
    match id {
        TipId::Filter => format!("Type {key} to filter this folder by name."),
        TipId::Search => format!("{key} searches subfolders and file contents too."),
        TipId::GoTo => format!("{key} finds recent and favorite folders by a few letters."),
    }
}

fn uses_of(id: TipId, cx: &App) -> u32 {
    match id {
        TipId::Filter => crate::usage::uses(&StartFilter, cx),
        TipId::Search => crate::usage::uses(&ShowSearch, cx),
        TipId::GoTo => crate::usage::uses(&ShowCommandPalette, cx),
    }
}

fn save_in_background(cx: &mut App) {
    let (state, file, previous) = {
        let tips = cx.global_mut::<Tips>();
        (tips.book.state().clone(), tips.file.clone(), tips.save.take())
    };
    let task = cx.background_spawn(async move {
        if let Some(previous) = previous {
            previous.await;
        }
        if let Err(e) = state.save(&file) {
            tracing::warn!("{}: {e}", file.display());
        }
    });
    cx.global_mut::<Tips>().save = Some(task);
}

/// Re-read `tips.json` into a fresh book, as a relaunch would (tests only).
#[cfg(test)]
pub fn reload_for_tests(cx: &mut App) {
    let file = cx.global::<Tips>().file.clone();
    let state = TipState::load(&file);
    let tips = cx.global_mut::<Tips>();
    tips.book = TipBook::new(state);
    tips.owner = None;
}

/// The tip card: keycap, one sentence, "Got it". Bottom right, above the bar.
pub fn render_tip(
    id: TipId,
    key: SharedString,
    theme: &crow_theme::Theme,
    window_width: f32,
    on_got_it: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let sentence = text(id, &key);
    let max_width = (window_width - 24.).min(440.);
    let surface = color(theme.role("surface"));
    let raised = color(theme.role("surface-raised"));
    let hover = color(theme.role("border"));
    let selection = color(theme.role("selection"));
    let fg = color(theme.role("fg"));
    div()
        .id("tip")
        .debug_selector(|| "tip".into())
        .role(Role::Status)
        .aria_label(format!("Tip: {sentence}"))
        .absolute()
        .right(px(12.))
        .bottom(px(crate::bottom_bar::HEIGHT + 8.))
        .min_w(px(280.))
        .max_w(px(max_width))
        .flex()
        .items_center()
        .gap(px(12.))
        .pt(px(10.))
        .pb(px(10.))
        .pl(px(14.))
        .pr(px(10.))
        .rounded(px(10.))
        .bg(surface)
        .border_1()
        .border_color(raised)
        .shadow_lg()
        .child(Keycap::text(key, KeycapWeight::Key).text_role("accent"))
        .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(fg).child(sentence))
        .child(
            div()
                .id("tip-got-it")
                .debug_selector(|| "tip-got-it".into())
                .role(Role::Button)
                .aria_label("Got it")
                .aria_description("Don't show this tip again")
                .flex_none()
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .bg(raised)
                .text_size(px(12.))
                .text_color(fg)
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(selection))
                .on_click(on_got_it)
                .child("Got it"),
        )
}
