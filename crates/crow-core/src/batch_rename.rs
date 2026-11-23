//! Batch rename rules: a pure, deterministic transform from the selected
//! entries to one preview row each. No filesystem access; the operation engine
//! validates targets, identities and volume name rules before anything moves.
//!
//! One rule runs at a time and applies to the original name, never to another
//! row's result. Numbering follows display order with a fixed +1 step.

use std::ops::Range;
use std::path::PathBuf;

use crow_fs::ops::check_name;
use unicode_segmentation::UnicodeSegmentation;

/// Where add text goes around the stem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddPosition {
    Before,
    After,
}

/// The rule the sheet is editing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenameKind {
    #[default]
    Replace,
    AddText,
    Number,
}

/// One selected entry frozen at invocation, in display order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameInput {
    pub source: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// Everything the form controls. Empty optional fields mean their default:
/// `start` 1 and no zero padding. Fields that belong to another rule are kept
/// so switching back does not lose what was typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameForm {
    pub kind: RenameKind,
    pub find: String,
    pub replace: String,
    pub add: String,
    pub add_position: AddPosition,
    pub base: String,
    pub start: String,
    pub padding: String,
    pub preserve_extension: bool,
}

impl Default for RenameForm {
    fn default() -> Self {
        Self {
            kind: RenameKind::Replace,
            find: String::new(),
            replace: String::new(),
            add: String::new(),
            add_position: AddPosition::After,
            base: String::new(),
            start: "1".to_owned(),
            padding: String::new(),
            preserve_extension: true,
        }
    }
}

/// One preview row. `destination_name` is the proposed directory-entry name,
/// even when the row is unchanged or invalid. `removed` and `inserted` are the
/// byte ranges that differ in the original and the destination (see
/// [`changed_spans`]); both are empty for an unchanged or invalid row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRow {
    pub source: PathBuf,
    pub original_name: String,
    pub is_dir: bool,
    pub destination_name: String,
    pub state: RowState,
    pub removed: Range<usize>,
    pub inserted: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowState {
    Unchanged,
    Ready,
    Invalid(String),
}

/// The pure preview plus one form-wide error (a blank field or an unreadable
/// number). With an error every row is invalid and there is nothing to submit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub rows: Vec<PreviewRow>,
    pub error: Option<String>,
}

impl Preview {
    pub fn changed(&self) -> usize {
        self.rows.iter().filter(|row| row.state == RowState::Ready).count()
    }

    /// Whether the rule itself produced at least one applicable change and no
    /// row is invalid.
    pub fn complete(&self) -> bool {
        self.error.is_none()
            && self.changed() > 0
            && !self.rows.iter().any(|row| matches!(row.state, RowState::Invalid(_)))
    }
}

/// Apply `form` to `inputs` in display order. Pure; the caller runs filesystem
/// validation separately.
pub fn preview(inputs: &[RenameInput], form: &RenameForm) -> Preview {
    let error = form_error(form);
    let numbers = if form.kind == RenameKind::Number { parse_numbers(form).ok() } else { Some((0, 0)) };
    let mut rows = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let invalid = |reason: String| PreviewRow {
            source: input.source.clone(),
            original_name: input.name.clone(),
            is_dir: input.is_dir,
            destination_name: input.name.clone(),
            state: RowState::Invalid(reason),
            removed: 0..0,
            inserted: 0..0,
        };
        if let Some(error) = &error {
            rows.push(invalid(error.clone()));
            continue;
        }
        let Some((start, padding)) = numbers else { unreachable!("form_error catches unreadable numbers") };
        let Some(number) = start.checked_add(index as u64) else {
            rows.push(invalid("the number is too large".to_owned()));
            continue;
        };
        let destination = match apply(input, form, number, padding) {
            Ok(name) => name,
            Err(reason) => {
                rows.push(invalid(reason));
                continue;
            }
        };
        let state = if destination == input.name {
            RowState::Unchanged
        } else {
            match check_name(&destination) {
                Ok(()) => RowState::Ready,
                Err(crow_fs::ops::OpError::InvalidName(reason)) => RowState::Invalid(reason),
                Err(other) => RowState::Invalid(other.to_string()),
            }
        };
        let (removed, inserted) =
            if state == RowState::Ready { changed_spans(&input.name, &destination) } else { (0..0, 0..0) };
        rows.push(PreviewRow {
            source: input.source.clone(),
            original_name: input.name.clone(),
            is_dir: input.is_dir,
            destination_name: destination,
            state,
            removed,
            inserted,
        });
    }
    Preview { rows, error }
}

/// The part of `old` that goes away and the part of `new` that replaces it:
/// what is left after trimming the longest common prefix and suffix, counted in
/// grapheme clusters so a combining mark is never split from its base.
pub fn changed_spans(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    if old == new {
        return (0..0, 0..0);
    }
    let old_clusters: Vec<(usize, &str)> = old.grapheme_indices(true).collect();
    let new_clusters: Vec<(usize, &str)> = new.grapheme_indices(true).collect();
    let prefix = old_clusters.iter().zip(&new_clusters).take_while(|(a, b)| a.1 == b.1).count();
    let limit = old_clusters.len().min(new_clusters.len()) - prefix;
    let suffix =
        old_clusters.iter().rev().zip(new_clusters.iter().rev()).take(limit).take_while(|(a, b)| a.1 == b.1).count();
    let span = |text: &str, clusters: &[(usize, &str)]| {
        let start = clusters.get(prefix).map_or(text.len(), |cluster| cluster.0);
        let end = clusters.get(clusters.len() - suffix).map_or(text.len(), |cluster| cluster.0);
        start..end.max(start)
    };
    (span(old, &old_clusters), span(new, &new_clusters))
}

fn form_error(form: &RenameForm) -> Option<String> {
    match form.kind {
        RenameKind::Replace if form.find.is_empty() => Some("Enter the text to find".to_owned()),
        RenameKind::AddText if form.add.is_empty() => Some("Enter the text to add".to_owned()),
        RenameKind::Number if form.base.trim().is_empty() => Some("Enter a base name".to_owned()),
        RenameKind::Number => {
            let start = match form.start.trim() {
                "" => Ok(1),
                text => text.parse::<u64>().map_err(|_| "Start must be a whole number".to_owned()),
            };
            let padding = match form.padding.trim() {
                "" | "0" => Ok(0),
                text => text.parse::<u32>().map_err(|_| "Zero padding must be 0–32".to_owned()).and_then(|padding| {
                    if padding <= 32 { Ok(padding as usize) } else { Err("Zero padding must be 0–32".to_owned()) }
                }),
            };
            start.err().or_else(|| padding.err())
        }
        _ => None,
    }
}

fn parse_numbers(form: &RenameForm) -> Result<(u64, usize), ()> {
    let start = match form.start.trim() {
        "" => 1,
        text => text.parse::<u64>().map_err(|_| ())?,
    };
    let padding = match form.padding.trim() {
        "" => 0,
        text => text.parse::<u32>().map_err(|_| ())? as usize,
    };
    Ok((start, padding.min(32)))
}

/// The extension split shared with the operation engine (packages included).
fn split(name: &str, is_dir: bool, preserve: bool) -> (&str, &str) {
    if preserve { crow_fs::ops::split_name(name, is_dir) } else { (name, "") }
}

fn apply(input: &RenameInput, form: &RenameForm, number: u64, padding: usize) -> Result<String, String> {
    let (stem, ext) = split(&input.name, input.is_dir, form.preserve_extension);
    let destination = match form.kind {
        RenameKind::Replace => {
            let replaced = stem.replace(&form.find, &form.replace);
            if replaced.is_empty() {
                return Err("the new name is empty".to_owned());
            }
            format!("{replaced}{ext}")
        }
        RenameKind::AddText => match form.add_position {
            AddPosition::Before => format!("{}{stem}{ext}", form.add),
            AddPosition::After => format!("{stem}{}{ext}", form.add),
        },
        RenameKind::Number => {
            let base = form.base.trim();
            let digits = number.to_string();
            let padded = if padding > digits.len() { format!("{number:0padding$}") } else { digits };
            format!("{base} {padded}{ext}")
        }
    };
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, is_dir: bool) -> RenameInput {
        RenameInput { source: PathBuf::from("/tmp").join(name), name: name.to_owned(), is_dir }
    }

    fn names(preview: &Preview) -> Vec<&str> {
        preview.rows.iter().map(|row| row.destination_name.as_str()).collect()
    }

    #[test]
    fn replace_is_case_sensitive_and_replaces_every_occurrence() {
        let form = RenameForm { find: "a".into(), replace: "x".into(), ..RenameForm::default() };
        let preview = preview(&[input("banana.txt", false), input("A-a.txt", false)], &form);
        assert_eq!(names(&preview), ["bxnxnx.txt", "A-x.txt"]);
        assert!(preview.rows.iter().all(|row| row.state == RowState::Ready));
    }

    #[test]
    fn add_text_keeps_the_extension_and_dotfiles_keep_their_leading_dot() {
        let form = RenameForm {
            kind: RenameKind::AddText,
            add: " draft".into(),
            add_position: AddPosition::After,
            ..RenameForm::default()
        };
        let after = preview(&[input("report.pdf", false), input(".bashrc", false)], &form);
        assert_eq!(names(&after), ["report draft.pdf", ".bashrc draft"]);
        let before = RenameForm { add_position: AddPosition::Before, ..form };
        let prepended = preview(&[input("report.pdf", false)], &before);
        assert_eq!(names(&prepended), [" draftreport.pdf"]);
    }

    #[test]
    fn folders_keep_their_whole_name_except_packages() {
        let form = RenameForm { find: "o".into(), replace: "0".into(), ..RenameForm::default() };
        let preview = preview(&[input("foo.bar", true), input("Tool.app", true), input("doc.txt", false)], &form);
        assert_eq!(
            names(&preview),
            ["f00.bar", "T00l.app", "d0c.txt"],
            "the folder keeps its dot; the package is split like a file"
        );
    }

    #[test]
    fn numbering_uses_display_order_start_and_padding() {
        let form = RenameForm {
            kind: RenameKind::Number,
            base: "Photo".into(),
            start: "7".into(),
            padding: "3".into(),
            ..RenameForm::default()
        };
        let preview = preview(&[input("b.jpg", false), input("a.jpg", false)], &form);
        assert_eq!(names(&preview), ["Photo 007.jpg", "Photo 008.jpg"]);
        assert!(preview.rows.iter().all(|row| row.state == RowState::Ready));
    }

    #[test]
    fn numbering_without_preserved_extension_drops_it() {
        let form = RenameForm {
            kind: RenameKind::Number,
            base: "Item".into(),
            preserve_extension: false,
            ..RenameForm::default()
        };
        let preview = preview(&[input("a.txt", false)], &form);
        assert_eq!(names(&preview), ["Item 1"]);
    }

    #[test]
    fn untouched_names_are_unchanged_not_ready() {
        let form = RenameForm { find: "zz".into(), replace: "x".into(), ..RenameForm::default() };
        let preview = preview(&[input("a.txt", false)], &form);
        assert_eq!(preview.rows[0].state, RowState::Unchanged);
        assert!(!preview.complete());
    }

    #[test]
    fn blank_fields_and_bad_numbers_are_errors() {
        for form in [
            RenameForm::default(),
            RenameForm { kind: RenameKind::AddText, ..RenameForm::default() },
            RenameForm { kind: RenameKind::Number, ..RenameForm::default() },
        ] {
            assert!(preview(&[input("a.txt", false)], &form).error.is_some());
        }
        let bad =
            RenameForm { kind: RenameKind::Number, base: "x".into(), start: "-1".into(), ..RenameForm::default() };
        assert!(preview(&[input("a.txt", false)], &bad).error.is_some());
    }

    #[test]
    fn numeric_overflow_invalidates_only_the_overflowing_rows() {
        let form = RenameForm {
            kind: RenameKind::Number,
            base: "x".into(),
            start: u64::MAX.to_string(),
            ..RenameForm::default()
        };
        let preview = preview(&[input("a.txt", false), input("b.txt", false)], &form);
        assert_eq!(preview.rows[0].state, RowState::Ready);
        assert!(matches!(preview.rows[1].state, RowState::Invalid(_)));
        assert!(!preview.complete());
    }

    #[test]
    fn slash_and_nul_targets_are_invalid() {
        let form = RenameForm { find: "a".into(), replace: "/".into(), ..RenameForm::default() };
        let slashed = preview(&[input("a.txt", false)], &form);
        assert!(matches!(slashed.rows[0].state, RowState::Invalid(_)));
        let form = RenameForm { find: "a".into(), replace: "\0".into(), ..RenameForm::default() };
        let nul = preview(&[input("a.txt", false)], &form);
        assert!(matches!(nul.rows[0].state, RowState::Invalid(_)));
    }

    #[test]
    fn spans_cover_what_differs_between_the_names() {
        let spans = |old, new| {
            let (removed, inserted) = changed_spans(old, new);
            (&old[removed], &new[inserted])
        };
        assert_eq!(spans("IMG_0412.jpg", "Trip-0412.jpg"), ("IMG_", "Trip-"));
        assert_eq!(spans("report.pdf", "report draft.pdf"), ("", " draft"));
        assert_eq!(spans("draft report.pdf", "report.pdf"), ("draft ", ""));
        assert_eq!(spans("a.txt", "Item 1"), ("a.txt", "Item 1"));
        assert_eq!(spans("photo.jpg", "Photo.jpg"), ("p", "P"));
        assert_eq!(spans("same.txt", "same.txt"), ("", ""));
        assert_eq!(changed_spans("same.txt", "same.txt"), (0..0, 0..0));
    }

    #[test]
    fn spans_never_split_a_combining_character() {
        // "e" + combining acute vs a plain "e": the whole cluster differs.
        let old = "cafe\u{301}.txt";
        let new = "cafe.txt";
        let (removed, inserted) = changed_spans(old, new);
        assert_eq!(&old[removed], "e\u{301}");
        assert_eq!(&new[inserted], "e");
    }

    #[test]
    fn rows_carry_spans_only_when_ready() {
        let form = RenameForm { find: "IMG_".into(), replace: "Trip-".into(), ..RenameForm::default() };
        let preview = preview(&[input("IMG_1.jpg", false), input("notes.txt", false)], &form);
        let ready = &preview.rows[0];
        assert_eq!(&ready.original_name[ready.removed.clone()], "IMG_");
        assert_eq!(&ready.destination_name[ready.inserted.clone()], "Trip-");
        assert_eq!(preview.rows[1].state, RowState::Unchanged);
        assert_eq!((preview.rows[1].removed.clone(), preview.rows[1].inserted.clone()), (0..0, 0..0));
    }
}
