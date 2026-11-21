//! The user's key overrides (`keys.toml`): parsing, key-text normalisation,
//! conflict rules and the comment-preserving writers. UI-free; `crow-app`
//! turns the result into GPUI bindings.
//!
//! ```toml
//! [finder]
//! go-to-folder = ["cmd-shift-g"]
//! [vim]
//! copy-files = ["y y"]
//! ```
//! A command id that is absent keeps its defaults; an empty list unbinds it.

use std::collections::BTreeMap;
use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::KeymapLayer;

pub const DEFAULT_KEYS: &str = r##"# Crow keyboard shortcuts. Change them in Settings > Keyboard or here; edits
# apply as soon as you save.
#
# Each line maps a command id to its keys. Commands you do not list keep their
# defaults, and an empty list ([]) removes the shortcut. Keys are written like
# "cmd-shift-g" (modifiers ctrl, cmd, alt, shift); a sequence is separated by
# spaces ("g g"). The [finder] table applies to the Finder keymap and [vim] to
# the Vim keymap.
#
# [finder]
# go-to-folder = ["cmd-shift-g"]
# new-tab = ["cmd-t", "cmd-alt-n"]
#
# [vim]
# copy-files = ["y y"]
"##;

/// Per-layer overrides: command id to normalised key sequences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyOverrides {
    pub finder: BTreeMap<String, Vec<String>>,
    pub vim: BTreeMap<String, Vec<String>>,
}

impl KeyOverrides {
    pub fn layer(&self, layer: KeymapLayer) -> &BTreeMap<String, Vec<String>> {
        match layer {
            KeymapLayer::Finder => &self.finder,
            KeymapLayer::Vim => &self.vim,
        }
    }
}

const MODIFIERS: [&str; 4] = ["ctrl", "cmd", "alt", "shift"];

fn canonical_modifier(name: &str) -> Option<&'static str> {
    Some(match name {
        "ctrl" | "control" => "ctrl",
        "cmd" | "command" | "super" => "cmd",
        "alt" | "option" | "opt" => "alt",
        "shift" => "shift",
        _ => return None,
    })
}

/// One keystroke in canonical form: modifiers in the order ctrl, cmd, alt,
/// shift, then the key, lower-case: `shift-cmd-G` becomes `cmd-shift-g`.
pub fn normalize_keystroke(text: &str) -> Result<String, String> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return Err("empty key".into());
    }
    let mut held = [false; 4];
    // Peel "<modifier>-" prefixes; what is left is the key, which may itself be "-".
    'peel: loop {
        for name in ["ctrl", "control", "cmd", "command", "super", "alt", "option", "opt", "shift"] {
            if let Some(tail) = rest.strip_prefix(name).and_then(|t| t.strip_prefix('-')) {
                if tail.is_empty() {
                    break;
                }
                let canon = canonical_modifier(name).expect("listed modifier");
                let slot = MODIFIERS.iter().position(|m| *m == canon).expect("modifier slot");
                if held[slot] {
                    return Err(format!("`{text}` repeats {canon}"));
                }
                held[slot] = true;
                rest = tail;
                continue 'peel;
            }
        }
        break;
    }
    if canonical_modifier(rest).is_some() {
        return Err(format!("`{text}` has only modifiers"));
    }
    let key = rest.to_lowercase();
    let single = key.chars().count() == 1;
    let named = matches!(
        key.as_str(),
        "up" | "down"
            | "left"
            | "right"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "enter"
            | "escape"
            | "tab"
            | "space"
            | "backspace"
            | "delete"
    ) || (key.starts_with('f') && key[1..].parse::<u8>().is_ok_and(|n| (1..=19).contains(&n)));
    if !single && !named {
        return Err(format!("`{text}`: unknown key `{rest}`"));
    }
    let mut out = String::new();
    for (slot, name) in MODIFIERS.iter().enumerate() {
        if held[slot] {
            out.push_str(name);
            out.push('-');
        }
    }
    out.push_str(&key);
    Ok(out)
}

/// A whitespace-separated sequence of keystrokes, each normalised.
pub fn normalize_sequence(text: &str) -> Result<String, String> {
    let parts: Vec<String> = text.split_whitespace().map(normalize_keystroke).collect::<Result<_, _>>()?;
    if parts.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(parts.join(" "))
}

/// True when two sequences cannot both fire: they are equal, or one is a
/// prefix of the other (Vim `g` against `g g`).
pub fn sequences_conflict(a: &str, b: &str) -> bool {
    let (a, b): (Vec<_>, Vec<_>) = (a.split_whitespace().collect(), b.split_whitespace().collect());
    let n = a.len().min(b.len());
    n > 0 && a[..n] == b[..n]
}

/// Why a key sequence cannot be assigned to any command, or `None`.
pub fn reserved_reason(sequence: &str) -> Option<&'static str> {
    Some(match sequence {
        "cmd-q" => "⌘Q quits the app",
        "cmd-h" => "⌘H hides the app",
        "cmd-alt-h" => "⌥⌘H hides the other apps",
        "cmd-m" => "⌘M minimises the window",
        "cmd-," => "⌘, opens Settings",
        "cmd-space" | "ctrl-space" | "cmd-alt-space" => "macOS uses this for Spotlight and input sources",
        "cmd-tab" | "cmd-shift-tab" | "cmd-`" => "macOS uses this to switch apps and windows",
        "escape" => "Escape cancels and closes things",
        _ => return None,
    })
}

/// Why `sequence` cannot be assigned in `layer`, or `None`. Reserved keys belong
/// to macOS or the app frame; in the Finder layer a bare letter or digit would
/// fight type-to-select (the Vim layer binds plain keys on purpose).
pub fn assign_problem(layer: KeymapLayer, sequence: &str) -> Option<String> {
    if let Some(reason) = reserved_reason(sequence) {
        return Some(reason.to_owned());
    }
    let bare = |step: &str| step.chars().count() == 1 && step.chars().all(|c| c.is_alphanumeric());
    if layer == KeymapLayer::Finder && sequence.split_whitespace().any(bare) {
        return Some("Plain letters and digits select files by typing; add ⌘, ⌃ or ⌥".to_owned());
    }
    None
}

/// `keys.toml` problems carry what is wrong; the file stays authoritative, so
/// a bad entry is reported, never silently dropped.
pub fn parse_keys(text: &str) -> Result<KeyOverrides, String> {
    let doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut out = KeyOverrides::default();
    for (name, item) in doc.iter() {
        let target = match name {
            "finder" => &mut out.finder,
            "vim" => &mut out.vim,
            other => return Err(format!("unknown table [{other}]; use [finder] or [vim]")),
        };
        let table = item.as_table().ok_or_else(|| format!("[{name}] must be a table"))?;
        for (command, item) in table.iter() {
            let array = item.as_array().ok_or_else(|| format!("[{name}] {command}: expected a list of keys"))?;
            let mut keys = Vec::new();
            for entry in array.iter() {
                let text = entry.as_str().ok_or_else(|| format!("[{name}] {command}: keys must be strings"))?;
                keys.push(normalize_sequence(text).map_err(|e| format!("[{name}] {command}: {e}"))?);
            }
            target.insert(command.to_owned(), keys);
        }
    }
    Ok(out)
}

/// Read `keys.toml`, writing the documented default on first launch.
pub fn load_keys(config_dir: &Path) -> Result<KeyOverrides, String> {
    let path = config_dir.join("keys.toml");
    if !path.exists() {
        crate::write_atomic(&path, DEFAULT_KEYS).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_keys(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn layer_table(doc: &mut DocumentMut, layer: KeymapLayer) -> Result<&mut Table, String> {
    let name = match layer {
        KeymapLayer::Finder => "finder",
        KeymapLayer::Vim => "vim",
    };
    let item = doc.entry(name).or_insert_with(|| Item::Table(Table::new()));
    item.as_table_mut().ok_or_else(|| format!("[{name}] must be a table"))
}

/// Set a command's keys (an empty list unbinds it).
pub fn set_keys(path: &Path, layer: KeymapLayer, command: &str, keys: &[String]) -> Result<(), String> {
    crate::edit_toml(path, DEFAULT_KEYS, |doc| {
        let table = layer_table(doc, layer)?;
        let mut array = Array::new();
        for key in keys {
            array.push(key.as_str());
        }
        table.insert(command, value(array));
        Ok(())
    })
}

/// Drop a command's override so it returns to its defaults.
pub fn reset_command(path: &Path, layer: KeymapLayer, command: &str) -> Result<(), String> {
    crate::edit_toml(path, DEFAULT_KEYS, |doc| {
        let table = layer_table(doc, layer)?;
        table.remove(command);
        if table.is_empty() {
            let name = match layer {
                KeymapLayer::Finder => "finder",
                KeymapLayer::Vim => "vim",
            };
            doc.remove(name);
        }
        Ok(())
    })
}

/// Drop every override of one layer, or of both with `None`.
pub fn reset_all(path: &Path, layer: Option<KeymapLayer>) -> Result<(), String> {
    crate::edit_toml(path, DEFAULT_KEYS, |doc| {
        for (name, l) in [("finder", KeymapLayer::Finder), ("vim", KeymapLayer::Vim)] {
            if layer.is_none_or(|only| only == l) {
                doc.remove(name);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystrokes_normalise_order_case_and_aliases() {
        assert_eq!(normalize_keystroke("shift-cmd-G").unwrap(), "cmd-shift-g");
        assert_eq!(normalize_keystroke("option-command-n").unwrap(), "cmd-alt-n");
        assert_eq!(normalize_keystroke("ctrl-cmd-s").unwrap(), "ctrl-cmd-s");
        assert_eq!(normalize_keystroke("cmd--").unwrap(), "cmd--");
        assert_eq!(normalize_keystroke("-").unwrap(), "-");
        assert_eq!(normalize_keystroke("F2").unwrap(), "f2");
        assert_eq!(normalize_keystroke("cmd-backspace").unwrap(), "cmd-backspace");
    }

    #[test]
    fn bad_keystrokes_say_why() {
        assert!(normalize_keystroke("").is_err());
        assert!(normalize_keystroke("cmd").is_err());
        assert!(normalize_keystroke("cmd-cmd-g").unwrap_err().contains("repeats"));
        assert!(normalize_keystroke("cmd-banana").unwrap_err().contains("unknown key"));
        assert!(normalize_keystroke("f40").is_err());
    }

    #[test]
    fn sequences_normalise_each_step() {
        assert_eq!(normalize_sequence("g  G").unwrap(), "g g");
        assert_eq!(normalize_sequence("shift-g").unwrap(), "shift-g");
        assert!(normalize_sequence("  ").is_err());
    }

    #[test]
    fn sequence_conflicts_are_equal_or_prefix() {
        assert!(sequences_conflict("g g", "g g"));
        assert!(sequences_conflict("g", "g g"));
        assert!(sequences_conflict("g g", "g"));
        assert!(!sequences_conflict("g g", "g d"));
        assert!(!sequences_conflict("y y", "d d"));
        assert!(!sequences_conflict("cmd-k", "cmd-shift-k"));
    }

    #[test]
    fn reserved_keys_carry_a_reason() {
        assert!(reserved_reason("cmd-q").unwrap().contains("quits"));
        assert!(reserved_reason("cmd-space").is_some());
        assert!(reserved_reason("cmd-k").is_none());
        assert!(reserved_reason("cmd-c").is_none(), "⌘C conflicts with Copy rather than being reserved");
    }

    #[test]
    fn plain_letters_are_refused_in_the_finder_layer_only() {
        assert!(assign_problem(KeymapLayer::Finder, "a").is_some());
        assert!(assign_problem(KeymapLayer::Finder, "g g").is_some());
        assert!(assign_problem(KeymapLayer::Vim, "g g").is_none());
        assert!(assign_problem(KeymapLayer::Finder, "cmd-a").is_none());
        assert!(assign_problem(KeymapLayer::Finder, "/").is_none());
        assert!(assign_problem(KeymapLayer::Finder, "f2").is_none());
        assert!(assign_problem(KeymapLayer::Vim, "cmd-q").is_some());
    }

    #[test]
    fn keys_toml_parses_both_layers() {
        let parsed =
            parse_keys("[finder]\ngo-to-folder = [\"shift-cmd-g\"]\nnew-tab = []\n[vim]\ncopy-files = [\"y y\"]\n")
                .unwrap();
        assert_eq!(parsed.finder["go-to-folder"], vec!["cmd-shift-g"]);
        assert!(parsed.finder["new-tab"].is_empty());
        assert_eq!(parsed.vim["copy-files"], vec!["y y"]);
        assert_eq!(parse_keys(DEFAULT_KEYS).unwrap(), KeyOverrides::default());
    }

    #[test]
    fn keys_toml_errors_name_the_entry() {
        assert!(parse_keys("[nope]\n").unwrap_err().contains("[nope]"));
        assert!(parse_keys("[finder]\nx = \"cmd-g\"\n").unwrap_err().contains("list of keys"));
        assert!(parse_keys("[finder]\nx = [\"cmd-banana\"]\n").unwrap_err().contains("finder] x"));
        assert!(parse_keys("[finder\n").is_err());
    }

    #[test]
    fn writers_keep_comments_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("crow-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keys.toml");
        std::fs::write(&path, "# mine\n[finder]\n# keep me\nnew-tab = [\"cmd-t\"]\n").unwrap();
        set_keys(&path, KeymapLayer::Finder, "go-to-folder", &["cmd-shift-g".into()]).unwrap();
        set_keys(&path, KeymapLayer::Vim, "copy-files", &["y y".into()]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine") && text.contains("# keep me"));
        let parsed = parse_keys(&text).unwrap();
        assert_eq!(parsed.finder.len(), 2);
        reset_command(&path, KeymapLayer::Finder, "go-to-folder").unwrap();
        assert_eq!(parse_keys(&std::fs::read_to_string(&path).unwrap()).unwrap().finder.len(), 1);
        reset_all(&path, Some(KeymapLayer::Vim)).unwrap();
        assert!(parse_keys(&std::fs::read_to_string(&path).unwrap()).unwrap().vim.is_empty());
        reset_all(&path, None).unwrap();
        assert_eq!(parse_keys(&std::fs::read_to_string(&path).unwrap()).unwrap(), KeyOverrides::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writers_create_the_file_from_the_documented_default() {
        let dir = std::env::temp_dir().join(format!("crow-keys-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keys.toml");
        set_keys(&path, KeymapLayer::Finder, "new-tab", &[]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Crow keyboard shortcuts"));
        assert!(parse_keys(&text).unwrap().finder["new-tab"].is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
