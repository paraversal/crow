//! ⌘L: the bottom bar's path slot becomes a field.
//!
//! `complete_typed_path` stays in `window.rs` because the palette shares it;
//! the bar's own rendering and hover state stay with the bottom bar.

use super::*;

/// ⌘L or a double-click on empty path space: the bottom bar's path slot becomes
/// a field. Escape and a focus loss put the segments back without navigating.
pub(super) struct PathEdit {
    pub(super) input: Entity<TextInput>,
    /// Where Escape returns focus, if the window had one when editing began.
    return_focus: Option<FocusHandle>,
    /// The bar error this field set for a missing path, if it is still the one shown.
    refused: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

impl CrowWindow {
    /// ⌘L: edit the path the bar shows in the bottom bar. A second ⌘L selects
    /// the field's text again; overlays and a rename own the screen, so they keep it.
    pub(super) fn edit_path(&mut self, _: &EditPath, window: &mut Window, cx: &mut Context<Self>) {
        if self.pane().read(cx).folder().is_none() {
            self.refuse_folder_command(cx);
            return;
        }
        if let Some(edit) = &self.path_edit {
            let input = edit.input.clone();
            let len = input.read(cx).text().len();
            input.update(cx, |input, cx| input.select_range(0..len, window, cx));
            let focus = input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            return;
        }
        if self.sheet.is_some()
            || self.why.is_some()
            || self.picker.is_some()
            || self.palette.is_some()
            || self.pane().read(cx).is_renaming()
        {
            return;
        }
        let typed = {
            let pane = self.pane().read(cx);
            let (target, _) = self.bar_target(pane, cx);
            home_relative(&target)
        };
        let input = cx.new(|cx| TextInput::new(typed.clone(), "Path", cx).bare().with_aria_label("Path"));
        input.update(cx, |input, cx| input.select_range(0..typed.len(), window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _input, event, window, cx| match event {
            TextInputEvent::Confirmed => this.confirm_path_edit(window, cx),
            TextInputEvent::Cancelled => this.close_path_edit(true, window, cx),
            TextInputEvent::CompletionRequested => this.complete_path_edit(window, cx),
            TextInputEvent::Changed => this.complete_path_preview(window, cx),
            TextInputEvent::BackspaceAtStart => {}
        });
        let focus = input.read(cx).focus_handle(cx);
        let blur = cx.on_focus_out(&focus, window, |this, _event, window, cx| {
            this.close_path_edit(false, window, cx);
        });
        let return_focus = window.focused(cx);
        self.path_edit =
            Some(PathEdit { input, return_focus, refused: None, _subscriptions: vec![subscription, blur] });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Leave edit mode and put the segments back. Escape returns focus to where it
    /// was; a click elsewhere and a focus loss leave focus where the user put it.
    pub(super) fn close_path_edit(&mut self, restore_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.path_edit.take() else { return };
        // The field replaced the path area, so its hover state is stale; the pill
        // fades back in on the next hover instead of sticking where the pointer left.
        self.path_hovered = false;
        // The refusal described the typed path; it goes with the field, unless
        // something newer replaced it.
        if let Some(generation) = edit.refused
            && cx.global::<Operations>().message_generation == generation
        {
            operations::clear_message(cx);
        }
        if restore_focus {
            match &edit.return_focus {
                Some(focus) => window.focus(focus, cx),
                None => self.focus_center(window, cx),
            }
        }
        cx.notify();
    }

    /// The Explorer Go chevron: the same confirmation as Return in the field.
    pub(super) fn path_edit_go(&mut self, _: &crate::actions::PathEditGo, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_path_edit(window, cx);
    }

    /// Start a typed-path lookup: the generation it must still own and the folder
    /// it resolves against.
    pub(super) fn begin_edit_lookup(&mut self, cx: &App) -> (u64, PathBuf) {
        self.edit_generation = self.edit_generation.wrapping_add(1);
        (self.edit_generation, self.pane().read(cx).current_dir(cx))
    }

    /// Whether a lookup's reply still answers the field: not superseded, and the
    /// active tab still shows the folder it was resolved against.
    pub(super) fn edit_lookup_current(&self, generation: u64, cwd: &Path, cx: &App) -> bool {
        self.edit_generation == generation && self.pane().read(cx).current_dir(cx) == cwd
    }

    /// Return in the path field, or the Explorer Go chevron: go to the folder, or
    /// reveal the file. A path that doesn't exist beeps and leaves the field open,
    /// with the reason in the bar.
    fn confirm_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.path_edit else { return };
        let input = edit.input.clone();
        let typed = input.read(cx).text().to_owned();
        let query = typed.clone();
        let (generation, cwd) = self.begin_edit_lookup(cx);
        let reply_cwd = cwd.clone();
        let home = cx.global::<Ui>().home.clone();
        let task = cx.background_spawn(async move {
            let cleaned = crow_core::clean_pasted_path(&query).unwrap_or_else(|| query.clone());
            let target = crow_core::resolve_typed(&cleaned, &cwd, &home);
            // io-ok: inside background_spawn
            match std::fs::metadata(&target) {
                Ok(meta) => {
                    // io-ok: inside background_spawn
                    let is_dir = meta.is_dir();
                    Ok((is_dir, target))
                }
                Err(error) => Err((query, error.to_string())),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(edit) = &this.path_edit else { return };
                if edit.input.read(cx).text() != typed || !this.edit_lookup_current(generation, &reply_cwd, cx) {
                    return;
                }
                match found {
                    Ok((is_dir, target)) => {
                        this.close_path_edit(false, window, cx);
                        if is_dir {
                            if this.pane().read(cx).current_dir(cx) != target {
                                this.pane().update(cx, |pane, cx| pane.navigate(target, cx));
                            }
                        } else {
                            this.pane().update(cx, |pane, cx| pane.reveal(target, cx));
                        }
                        this.focus_center(window, cx);
                        cx.notify();
                    }
                    Err((typed, error)) => {
                        crate::feedback::refuse(crate::feedback::RefuseTarget::Field(&input), window, cx);
                        operations::message(format!("Can't open “{typed}”: {error}"), true, cx);
                        if let Some(edit) = &mut this.path_edit {
                            edit.refused = Some(cx.global::<Operations>().message_generation);
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Tab in the path field: complete the last component from disk, like ⌘K.
    fn complete_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.path_edit else { return };
        let input = edit.input.clone();
        let query = input.read(cx).text().to_owned();
        let (generation, cwd) = self.begin_edit_lookup(cx);
        let task = self.complete_typed_path(query.clone(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let completed = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(edit) = &this.path_edit else { return };
                if edit.input.read(cx).text() != query || !this.edit_lookup_current(generation, &cwd, cx) {
                    return;
                }
                if let Some(completed) = completed {
                    input.update(cx, |input, cx| {
                        input.set_text(completed, window, cx);
                        input.set_completion(None, cx);
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Recompute the field's muted completion preview as the typed path changes.
    fn complete_path_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.path_edit else { return };
        let input = edit.input.clone();
        let typed = input.read(cx).text().to_owned();
        // The muted preview never bumps the generation: a later keystroke's
        // preview must not cancel a Return already on its way.
        let cwd = self.pane().read(cx).current_dir(cx);
        let task = self.complete_typed_path(typed.clone(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let completed = task.await;
            this.update_in(cx, |this, window, cx| {
                let Some(edit) = &this.path_edit else { return };
                if edit.input.read(cx).text() != typed || this.pane().read(cx).current_dir(cx) != cwd {
                    return;
                }
                let suffix = completed.and_then(|completed| {
                    let prefix = completed.get(..typed.len())?;
                    prefix.eq_ignore_ascii_case(&typed).then(|| SharedString::from(&completed[typed.len()..]))
                });
                input.update(cx, |input, cx| input.set_completion(suffix, cx));
                let _ = window;
            })
            .ok();
        })
        .detach();
    }
}
