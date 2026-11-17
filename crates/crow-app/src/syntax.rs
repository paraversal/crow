//! A small, dependency-free highlighter for the preview's text: comments,
//! strings, numbers, keywords and types, one line at a time. It colours the
//! languages Crow's own files use; multi-line strings and block comments are
//! left plain rather than tracked across lines.

use std::ops::Range;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Rust,
    Toml,
    Json,
    JavaScript,
    Python,
    Shell,
    Plain,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Language::Rust,
            "toml" => Language::Toml,
            "json" => Language::Json,
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Language::JavaScript,
            "py" => Language::Python,
            "sh" | "bash" | "zsh" | "fish" | "command" => Language::Shell,
            _ => Language::Plain,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Language::Rust => &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
                "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
                "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
                "where", "while", "yield",
            ],
            Language::Toml => &["true", "false"],
            Language::Json => &["true", "false", "null"],
            Language::JavaScript => &[
                "async",
                "await",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "default",
                "delete",
                "do",
                "else",
                "export",
                "extends",
                "false",
                "finally",
                "for",
                "from",
                "function",
                "if",
                "import",
                "in",
                "instanceof",
                "interface",
                "let",
                "new",
                "null",
                "of",
                "return",
                "static",
                "super",
                "switch",
                "this",
                "throw",
                "true",
                "try",
                "type",
                "typeof",
                "undefined",
                "var",
                "void",
                "while",
                "yield",
            ],
            Language::Python => &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else",
                "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None",
                "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try", "while", "with", "yield",
            ],
            Language::Shell => &[
                "case", "do", "done", "elif", "else", "esac", "export", "false", "fi", "for", "function", "if", "in",
                "local", "readonly", "return", "then", "true", "while",
            ],
            Language::Plain => &[],
        }
    }

    fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::Rust | Language::JavaScript => Some("//"),
            Language::Toml | Language::Python | Language::Shell => Some("#"),
            Language::Json | Language::Plain => None,
        }
    }

    /// Rust's `'a` is a lifetime, not a character; the other languages use `'`
    /// for strings, and an unterminated one stops at the end of the line.
    fn single_quotes(self) -> bool {
        self != Language::Rust
    }

    /// Capitalized identifiers read as types in these languages.
    fn types(self) -> bool {
        matches!(self, Language::Rust | Language::JavaScript | Language::Python)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Comment,
    Str,
    Number,
    Keyword,
    Type,
}

impl Kind {
    /// The theme role the preview draws this kind with.
    pub fn role(self) -> &'static str {
        match self {
            Kind::Comment => "fg-subtle",
            Kind::Str => "success",
            Kind::Number => "warning",
            Kind::Keyword => "accent",
            Kind::Type => "info",
        }
    }
}

/// The highlighted ranges of one line, sorted and non-overlapping.
pub fn spans(line: &str, language: Language) -> Vec<(Range<usize>, Kind)> {
    let mut spans = Vec::new();
    if language == Language::Plain {
        return spans;
    }
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        if let Some(marker) = language.line_comment()
            && comment_starts(line, i, marker)
        {
            spans.push((i..line.len(), Kind::Comment));
            break;
        }
        let c = rest.chars().next().expect("i is a char boundary");
        if c == '"' || (c == '\'' && language.single_quotes()) {
            let end = string_end(line, i, c);
            spans.push((i..end, Kind::Str));
            i = end;
            continue;
        }
        if c.is_ascii_digit() && line[..i].chars().next_back().is_none_or(|p| !is_ident(p)) {
            let end = number_end(line, i);
            spans.push((i..end, Kind::Number));
            i = end;
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            let mut end = i + c.len_utf8();
            while end < line.len() {
                let ch = line[end..].chars().next().expect("a char boundary");
                if !is_ident(ch) {
                    break;
                }
                end += ch.len_utf8();
            }
            let word = &line[start..end];
            if language.keywords().contains(&word) {
                spans.push((start..end, Kind::Keyword));
            } else if language.types() && word.chars().next().is_some_and(char::is_uppercase) {
                spans.push((start..end, Kind::Type));
            }
            i = end;
            continue;
        }
        i += c.len_utf8();
    }
    spans
}

/// `#` starts a comment only at a word boundary; `//` does not need one.
fn comment_starts(line: &str, i: usize, marker: &str) -> bool {
    line[i..].starts_with(marker)
        && (marker != "#" || i == 0 || line[..i].chars().next_back().is_some_and(char::is_whitespace))
}

fn string_end(line: &str, start: usize, quote: char) -> usize {
    let mut i = start + quote.len_utf8();
    let mut escaped = false;
    while i < line.len() {
        let c = line[i..].chars().next().expect("a char boundary");
        i += c.len_utf8();
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return i;
        }
    }
    line.len()
}

fn number_end(line: &str, start: usize) -> usize {
    let mut end = start;
    for c in line[start..].chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            end += c.len_utf8();
        } else if (c == '+' || c == '-') && line.as_bytes().get(end.wrapping_sub(1)) == Some(&b'e') {
            end += 1;
        } else {
            break;
        }
    }
    end
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of<'a>(line: &'a str, span: &(Range<usize>, Kind)) -> &'a str {
        &line[span.0.clone()]
    }

    fn find(line: &str, language: Language, wanted: &str, kind: Kind) -> bool {
        spans(line, language).iter().any(|span| text_of(line, span) == wanted && span.1 == kind)
    }

    #[test]
    fn rust_colors_keywords_types_strings_numbers_and_comments() {
        let line = r#"pub fn load(path: &Path) -> Result<Self, Error> { let n = 0x2a; // note"#;
        assert!(find(line, Language::Rust, "pub", Kind::Keyword));
        assert!(find(line, Language::Rust, "fn", Kind::Keyword));
        assert!(find(line, Language::Rust, "Path", Kind::Type));
        assert!(find(line, Language::Rust, "Self", Kind::Keyword), "Self is a keyword");
        assert!(find(line, Language::Rust, "Error", Kind::Type));
        assert!(find(line, Language::Rust, "0x2a", Kind::Number));
        assert!(find(line, Language::Rust, "// note", Kind::Comment));

        let string = r#"let s = "hi \"there\"";"#;
        assert!(find(string, Language::Rust, r#""hi \"there\"""#, Kind::Str));
        assert!(!find(string, Language::Rust, "hi", Kind::Str), "the whole literal is one span");
    }

    #[test]
    fn rust_lifetimes_are_not_strings() {
        let line = "fn f<'a>(x: &'a str) -> &'a str { x }";
        assert!(!spans(line, Language::Rust).iter().any(|span| span.1 == Kind::Str), "no string spans");
        assert!(find(line, Language::Rust, "fn", Kind::Keyword));
    }

    #[test]
    fn toml_colors_values_and_comments_but_not_keys() {
        let line = r#"terminal = "Terminal" # the app"#;
        assert!(find(line, Language::Toml, r#""Terminal""#, Kind::Str));
        assert!(find(line, Language::Toml, "# the app", Kind::Comment));
        assert!(!find(line, Language::Toml, "terminal", Kind::Keyword), "a key stays plain");

        let flag = "show_preview = true # on";
        assert!(find(flag, Language::Toml, "true", Kind::Keyword));
        let glued = "key = a#b";
        assert!(!spans(glued, Language::Toml).iter().any(|span| span.1 == Kind::Comment), "needs a word boundary");
    }

    #[test]
    fn json_and_javascript_values() {
        assert!(find(r#"{"ok": true, "n": 12}"#, Language::Json, "true", Kind::Keyword));
        assert!(find(r#"{"ok": true, "n": 12}"#, Language::Json, "12", Kind::Number));
        let js = "export function f() { return 'x'; } // done";
        assert!(find(js, Language::JavaScript, "export", Kind::Keyword));
        assert!(find(js, Language::JavaScript, "'x'", Kind::Str));
        assert!(find(js, Language::JavaScript, "// done", Kind::Comment));
    }

    #[test]
    fn python_and_shell() {
        let py = "def f(x):  # note\n    return None";
        let first = py.lines().next().expect("a line");
        assert!(find(first, Language::Python, "def", Kind::Keyword));
        assert!(find(first, Language::Python, "# note", Kind::Comment));
        assert!(find("return None", Language::Python, "None", Kind::Keyword));

        let sh = "if [ -f \"$1\" ]; then echo hi; fi # end";
        assert!(find(sh, Language::Shell, "if", Kind::Keyword));
        assert!(find(sh, Language::Shell, "\"$1\"", Kind::Str));
        assert!(find(sh, Language::Shell, "# end", Kind::Comment));
    }

    #[test]
    fn non_ascii_ranges_stay_on_char_boundaries() {
        let line = "let café = \"héllo\"; // naïve";
        for (range, _) in spans(line, Language::Rust) {
            assert!(line.is_char_boundary(range.start) && line.is_char_boundary(range.end), "{range:?}");
        }
        assert!(find(line, Language::Rust, "\"héllo\"", Kind::Str));
        assert!(find(line, Language::Rust, "// naïve", Kind::Comment));
    }

    #[test]
    fn an_unterminated_string_reaches_the_line_end() {
        let line = "let s = \"open";
        assert!(find(line, Language::Rust, "\"open", Kind::Str));
    }

    #[test]
    fn plain_files_have_no_spans() {
        assert!(spans("pub fn main() {}", Language::Plain).is_empty());
        assert_eq!(Language::from_path(Path::new("notes.txt")), Language::Plain);
        assert_eq!(Language::from_path(Path::new("Cargo.TOML")), Language::Toml);
        assert_eq!(Language::from_path(Path::new("lib.rs")), Language::Rust);
    }

    #[test]
    fn spans_are_sorted_and_disjoint() {
        for line in [
            r#"pub fn load(path: &Path) -> Result<Self, Error> { let n = 0x2a; // note"#,
            r#"terminal = "Terminal" # the app"#,
            "if [ -f \"$1\" ]; then echo hi; fi # end",
        ] {
            let spans = spans(line, Language::from_path(Path::new("x.rs")));
            for pair in spans.windows(2) {
                assert!(pair[0].0.end <= pair[1].0.start, "sorted and disjoint: {pair:?}");
            }
        }
    }
}
