//! The `/` filter's parts: the field inside the focused column (or the list
//! frame), the search footer, and the no-matches block. Split out of `pane.rs`
//! so the pane's own render code stays readable.

use std::time::Instant;

use crow_core::motion::{self, Effective};
use gpui::{
    AnyElement, App, Context, CursorStyle, Focusable, IntoElement, MouseButton, Role, Window, div, prelude::*, px,
};

use crate::actions::{ChooseTags, ShowSearch};
use crate::chrome::{Keycap, KeycapWeight};
use crate::icons::{Glyph, glyph};
use crate::keys::{self, KeyScope};
use crate::pane::Pane;
use crate::style::{color, color_alpha, display_name, field_ring};
use crate::text_input::Cancel;
use crate::ui::Ui;

/// The filter row opening or closing. Closing keeps the query and count so a
/// picture of the field can shrink away after the real one is gone.
#[derive(Clone)]
pub(super) struct FilterMotion {
    started: Instant,
    opening: bool,
    ghost: Option<(String, String)>,
}

impl FilterMotion {
    pub(super) fn opening(cx: &App) -> Option<Self> {
        (cx.global::<Ui>().feedback.mode != Effective::Off).then(|| FilterMotion {
            started: crate::clock::now(),
            opening: true,
            ghost: None,
        })
    }

    fn closing(query: String, count: String, cx: &App) -> Option<Self> {
        (cx.global::<Ui>().feedback.mode == Effective::Full).then(|| FilterMotion {
            started: crate::clock::now(),
            opening: false,
            ghost: Some((query, count)),
        })
    }
}

/// How tall the row is (share of its full height) and how visible at eased
/// progress `t`: Full opens and closes the height with the fade; Low opens
/// with a fade and a full-height row (gaps snap).
pub(super) fn row_look(mode: Effective, opening: bool, t: f32) -> (f32, f32) {
    let f = if opening { t } else { 1.0 - t };
    match mode {
        Effective::Full => (f, f),
        _ => (1.0, f),
    }
}

/// Which frame the field sits in; the margins line it up with that frame's rows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterPlace {
    Column,
    List,
}

impl Pane {
    /// What the row leaves behind when the filter closes, for its shrink.
    pub(super) fn filter_ghost(&self, cx: &App) -> Option<FilterMotion> {
        let filtering = self.filtering.as_ref()?;
        let folder = self.focused_folder().read(cx);
        let count = format!("{} of {}", folder.row_count(), folder.entries().len());
        FilterMotion::closing(filtering.input.read(cx).text().to_owned(), count, cx)
    }

    /// The field, when a filter is open: accent `/`, query, "n of N", Outline `esc`.
    pub(super) fn render_filter_field(
        &self,
        place: FilterPlace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let mode = cx.global::<Ui>().feedback.mode;
        let full_height = if place == FilterPlace::List { 36. } else { 32. };
        let progress = self.filter_motion.as_ref().and_then(|m| {
            motion::progress(mode, m.started, crate::clock::now(), motion::ms::FILTER_ROW, motion::ms::LOW_IN)
        });
        if progress.is_some() {
            window.request_animation_frame();
        }
        let Some(filtering) = self.filtering.as_ref() else {
            // Closing: a still picture of the field shrinks away (Full only).
            let (m, t) = self.filter_motion.as_ref().zip(progress)?;
            let (query, count) = m.ghost.as_ref()?;
            let theme = &cx.global::<Ui>().theme;
            let (height, opacity) = row_look(mode, false, t);
            let picture = div()
                .h(px(28.))
                .rounded(px(6.))
                .bg(color(theme.role("surface")))
                .pl(px(9.))
                .pr(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .when(place == FilterPlace::Column, |field| field.mx_1().mb_1())
                .when(place == FilterPlace::List, |field| field.mx_2().my_1())
                .text_size(px(13.))
                .text_color(color(theme.role("fg")))
                .child(div().text_color(color(theme.role("accent"))).font_weight(gpui::FontWeight::SEMIBOLD).child("/"))
                .child(div().flex_1().min_w_0().child(query.clone()))
                .child(div().text_size(px(12.)).text_color(color(theme.role("fg-muted"))).child(count.clone()));
            return Some(
                div()
                    .h(px(full_height * height))
                    .flex_none()
                    .overflow_hidden()
                    .opacity(opacity)
                    .debug_selector(|| "filter-ghost".into())
                    .child(picture)
                    .into_any_element(),
            );
        };
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let (shown, total, dir, tags, scanning, errors) = {
            let folder = self.focused_folder().read(cx);
            (
                folder.row_count(),
                folder.entries().len(),
                folder.dir().to_path_buf(),
                folder.tag_filter().to_vec(),
                folder.tag_scanning(),
                folder.tag_errors(),
            )
        };
        let name = display_name(&dir);
        let input = filtering.input.clone();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        // Transparent when the list has focus, so only the place keys go is accented.
        let border = if focused { field_ring(theme) } else { color_alpha(theme.role("accent"), 0) };
        let esc = Keycap::for_action(&Cancel, KeyScope::Filter, KeycapWeight::Outline, cx);
        let tag_label = match tags.len() {
            0 => "Tags".to_owned(),
            1 => tags[0].clone(),
            count => format!("{count} tags"),
        };
        let tag_label = if scanning { format!("{tag_label}…") } else { tag_label };
        let tag_aria = if tags.is_empty() {
            "Filter by tag".to_owned()
        } else {
            format!("Filter by tag; selected: {}", tags.join(", "))
        };
        let tag_control = div()
            .id("filter-tags")
            .debug_selector(|| "filter-tags".into())
            .role(Role::Button)
            .aria_label(tag_aria)
            .flex_none()
            .h(px(22.))
            .max_w(px(150.))
            .px(px(6.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(12.))
            .when(tags.is_empty(), |control| control.text_color(color(theme.role("fg-muted"))))
            .when(!tags.is_empty(), |control| {
                control.bg(color_alpha(theme.role("accent"), 36)).text_color(color(theme.role("accent")))
            })
            .hover(|style| style.bg(color(theme.role("surface-raised"))))
            .on_click(cx.listener(|_, _, window, cx| {
                window.dispatch_action(Box::new(ChooseTags), cx);
            }))
            .child(tag_label);
        let mut count = format!("{shown} of {total}");
        if scanning {
            count.push('…');
        }
        if errors > 0 {
            count.push_str(&format!(" · {errors} unreadable"));
        }
        let field = div()
            .id("filter-field")
            .debug_selector(|| "filter-field".into())
            .role(Role::Group)
            .aria_label(format!("Filter {name}"))
            // keys::context_stack (KeyScope::Filter) mirrors this.
            .key_context("Filter")
            .h(px(28.))
            .rounded(px(6.))
            .bg(color(theme.role("surface")))
            .pl(px(9.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(7.))
            .border_2()
            .border_color(border)
            .when(place == FilterPlace::Column, |field| field.mx_1().mb_1())
            .when(place == FilterPlace::List, |field| field.mx_2().my_1())
            .on_action(cx.listener(Self::filter_focus_results))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(filtering) = &this.filtering {
                    let focus = filtering.input.read(cx).focus_handle(cx);
                    window.focus(&focus, cx);
                }
            }))
            // The press belongs to the field, never to the column below it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color(theme.role("accent")))
                    .child("/"),
            )
            .child(div().flex_1().min_w_0().child(input))
            .child(tag_control)
            .child(
                div()
                    .id("filter-count")
                    .role(Role::Status)
                    .aria_label(if self.filter_announce.is_empty() {
                        format!("{shown} of {total} shown")
                    } else {
                        self.filter_announce.to_string()
                    })
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(color(theme.role("fg-muted")))
                    .child(count),
            )
            .children(esc);
        // Opening: the row grows and fades in (Low fades). The field and its caret
        // are live from the first frame, so a letter typed right away lands.
        if let Some((_, t)) = self.filter_motion.as_ref().zip(progress).filter(|(m, _)| m.opening) {
            let (height, opacity) = row_look(mode, true, t);
            return Some(
                div()
                    .flex_none()
                    .overflow_hidden()
                    .when(height < 1.0, |wrap| wrap.h(px(full_height * height)))
                    .opacity(opacity)
                    .child(field)
                    .into_any_element(),
            );
        }
        Some(field.into_any_element())
    }

    /// "“vi” in all of src ⌘F": shown while the filter has matches to go deeper on.
    pub(super) fn render_filter_footer(&self, place: FilterPlace, cx: &mut Context<Self>) -> Option<AnyElement> {
        let filtering = self.filtering.as_ref()?;
        let query = filtering.input.read(cx).text().to_owned();
        if query.is_empty() {
            return None;
        }
        let (shown, dir) = {
            let folder = self.focused_folder().read(cx);
            (folder.row_count(), folder.dir().to_path_buf())
        };
        if shown == 0 {
            return None;
        }
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let name = display_name(&dir);
        let label = format!("\u{201c}{query}\u{201d} in all of {name}");
        let spoken = keys::spoken_for(&ShowSearch, KeyScope::Filter, cx).unwrap_or_else(|| "Command F".into());
        let keycap = Keycap::for_action(&ShowSearch, KeyScope::Filter, KeycapWeight::Outline, cx);
        let footer = div()
            .id("filter-search-footer")
            .debug_selector(|| "filter-search-footer".into())
            .role(Role::Button)
            .aria_label(format!("Search {label}, {spoken}"))
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .border_t_1()
            .border_color(color(theme.role("border")))
            .text_size(px(12.))
            .text_color(color(theme.role("fg-muted")))
            .cursor(CursorStyle::PointingHand)
            .hover(move |style| style.bg(color(theme.role("surface"))))
            .when(place == FilterPlace::Column, |footer| footer.mx_1())
            .when(place == FilterPlace::List, |footer| footer.mx_2())
            .on_click(cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(ShowSearch), cx)))
            .child(glyph(Glyph::Search, 13., "fg-muted", theme))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
            .children(keycap);
        Some(footer.into_any_element())
    }

    /// The D state: nothing matches, with the way out (⌘F) and why the folder may be empty.
    pub(super) fn render_no_matches(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let query = self.filter_query(cx)?;
        let (name, tags) = {
            let folder = self.focused_folder().read(cx);
            let tags = folder.tag_filter().to_vec();
            if query.is_empty() && tags.is_empty() {
                return None;
            }
            let shown = folder.row_count();
            if !shows_no_matches(
                !query.is_empty() || !tags.is_empty(),
                shown,
                folder.is_loading() || folder.tag_scanning(),
                folder.error().is_some(),
            ) {
                return None;
            }
            (display_name(folder.dir()), tags)
        };
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let (fg, muted, accent) = (theme.role("fg"), theme.role("fg-muted"), theme.role("accent"));
        let text = if query.is_empty() {
            if tags.len() == 1 {
                format!("Nothing in {name} is tagged \u{201c}{}\u{201d}", tags[0])
            } else {
                format!("Nothing in {name} has every tag: {}", tags.join(", "))
            }
        } else {
            format!("Nothing in {name} matches \u{201c}{query}\u{201d}")
        };
        let spoken = keys::spoken_for(&ShowSearch, KeyScope::Filter, cx).unwrap_or_else(|| "Command F".into());
        let keycap = Keycap::for_action(&ShowSearch, KeyScope::Filter, KeycapWeight::Outline, cx)
            .map(|keycap| keycap.border("fg-subtle"));
        let note = hidden_note(self.show_hidden);
        let block = div()
            .id("filter-no-matches")
            .debug_selector(|| "filter-no-matches".into())
            .role(Role::Group)
            .aria_label("No matches")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .pt(px(22.))
            .px(px(10.))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(10.))
            .child(
                div()
                    .id("filter-no-match-text")
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .text_color(color(fg))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(text),
            )
            .child(
                div()
                    .id("filter-search-subfolders")
                    .debug_selector(|| "filter-search-subfolders".into())
                    .role(Role::Button)
                    .aria_label(format!("Search subfolders too, {spoken}"))
                    .h(px(26.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .bg(color(theme.role("surface-raised")))
                    .text_color(color(fg))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |style| style.bg(color_alpha(accent, 0x33)))
                    .active(move |style| style.bg(color_alpha(accent, 0x55)))
                    .on_click(cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(ShowSearch), cx)))
                    .child("Search subfolders too")
                    .children(keycap),
            )
            .child(
                div()
                    .id("filter-hidden-note")
                    .debug_selector(|| "filter-hidden-note".into())
                    .text_size(px(12.))
                    .text_color(color(muted))
                    .child(note),
            );
        Some(block.into_any_element())
    }
}

/// The D state replaces the list only when a non-empty filter has settled on a folder.
fn shows_no_matches(constrained: bool, shown: usize, loading: bool, error: bool) -> bool {
    constrained && shown == 0 && !loading && !error
}

/// Why the folder looks empty, and the way out that always works.
fn hidden_note(show_hidden: bool) -> &'static str {
    if show_hidden { "Esc clears" } else { "Hidden files are off \u{00b7} Esc clears" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_matches_waits_for_a_settled_listing() {
        assert!(shows_no_matches(true, 0, false, false));
        assert!(!shows_no_matches(true, 0, true, false), "still loading: keep the area quiet");
        assert!(!shows_no_matches(true, 0, false, true), "the folder's own error is shown instead");
        assert!(!shows_no_matches(true, 1, false, false), "there is a match");
        assert!(!shows_no_matches(false, 0, false, false), "no constraint never shows D");
    }

    #[test]
    fn hidden_note_names_the_blocker_when_hidden_files_are_off() {
        assert_eq!(hidden_note(false), "Hidden files are off \u{00b7} Esc clears");
        assert_eq!(hidden_note(true), "Esc clears");
    }
}
