//! Finder tag values: identity, validation and matching.
//!
//! Finder names are case-sensitive but NSString's equality is canonical, and
//! Spotlight's index normalizes NFC to NFD. Identity here is therefore NFD and
//! case-sensitive, so the direct metadata read, the walker and Spotlight agree
//! on what "the same tag" means.

use unicode_normalization::UnicodeNormalization;

/// More selected tags than this is a filter nobody can read, and a query with
/// more AND terms than Spotlight should be asked for.
pub const MAX_TAGS: usize = 16;
/// Finder tag values are short labels, not paths or prose.
pub const MAX_TAG_LEN: usize = 128;

/// The comparison form of a tag value. ASCII is unchanged.
pub fn normalize_tag(value: &str) -> String {
    if value.is_ascii() { value.to_owned() } else { value.nfd().collect() }
}

/// Validate one tag value and return its NFD form.
pub fn validate_tag(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("tag is empty".into());
    }
    if value.chars().count() > MAX_TAG_LEN {
        return Err(format!("tag is longer than {MAX_TAG_LEN} characters"));
    }
    if value.chars().any(|ch| ch == '\n' || ch == '\r' || ch.is_control()) {
        return Err(format!("\"{}\" contains a line break or control character", value.escape_debug()));
    }
    Ok(normalize_tag(value))
}

/// Validate and normalize a set of selected tags, keeping the first spelling of
/// each value and dropping duplicates. Order is preserved: it is the user's
/// selection order, not a sorted set.
pub fn normalize_tags<I, S>(values: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for value in values {
        let normalized = validate_tag(value.as_ref())?;
        if !out.contains(&normalized) {
            out.push(normalized);
        }
        if out.len() > MAX_TAGS {
            return Err(format!("use at most {MAX_TAGS} tags"));
        }
    }
    Ok(out)
}

/// Whether `entry_tags` (any spelling) contains every wanted value (NFD).
/// Unreadable metadata is represented by the caller as an empty list, so a
/// positive predicate fails there, as the plan requires.
pub fn tags_contain_all(entry_tags: &[String], wanted: &[String]) -> bool {
    wanted.iter().all(|want| {
        let want = normalize_tag(want);
        entry_tags.iter().any(|tag| tag == &want || normalize_tag(tag) == want)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_identity_is_nfd_and_case_stays_significant() {
        let composed = normalize_tag("Café");
        let decomposed = normalize_tag("Cafe\u{0301}");
        assert_eq!(composed, decomposed, "NFC and NFD are the same Finder tag");
        assert_ne!(normalize_tag("Work"), normalize_tag("work"), "Finder tag names are case-sensitive");
        assert_eq!(normalize_tag("ascii"), "ascii");
    }

    #[test]
    fn validation_rejects_empty_control_and_long_values() {
        assert_eq!(validate_tag("  Work  ").unwrap(), "Work");
        assert!(validate_tag("").is_err());
        assert!(validate_tag("two\nlines").is_err());
        assert!(validate_tag(&"x".repeat(MAX_TAG_LEN + 1)).is_err());
        assert_eq!(normalize_tags(["Work", "work", "Work"]).unwrap(), ["Work", "work"]);
        let too_many: Vec<String> = (0..=MAX_TAGS).map(|index| format!("t{index}")).collect();
        assert!(normalize_tags(too_many).is_err());
    }

    #[test]
    fn all_selected_tags_must_be_present() {
        let entry = vec!["Work".to_owned(), "Cafe\u{0301}".to_owned()];
        assert!(tags_contain_all(&entry, &["Work".to_owned()]));
        assert!(tags_contain_all(&entry, &["Work".to_owned(), "Café".to_owned()]));
        assert!(!tags_contain_all(&entry, &["review".to_owned()]));
        assert!(!tags_contain_all(&[], &["Work".to_owned()]), "no tags does not satisfy a positive predicate");
    }
}
