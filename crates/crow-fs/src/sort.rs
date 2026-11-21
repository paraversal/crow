use std::cmp::Ordering;

/// Case-insensitive natural order (`file2` before `file10`). Names equal under
/// natural order fall back to case-insensitive, then byte order, so the order is total.
///
/// Sorting 100k names calls this millions of times, so it never allocates on the
/// common path: ASCII is compared byte by byte, digit runs as slices.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        let (cx, cy) = (x[i], y[j]);
        if cx.is_ascii_digit() && cy.is_ascii_digit() {
            let (start_x, start_y) = (i, j);
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }
            let (n, m) = (trim_zeros(&x[start_x..i]), trim_zeros(&y[start_y..j]));
            let ord = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
            if ord != Ordering::Equal {
                return ord;
            }
        } else if cx.is_ascii() && cy.is_ascii() {
            let ord = cx.to_ascii_lowercase().cmp(&cy.to_ascii_lowercase());
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        } else {
            // i and j always sit on char boundaries: they only advance by whole
            // ASCII bytes, digit runs or decoded chars.
            let (Some(ca), Some(cb)) = (a[i..].chars().next(), b[j..].chars().next()) else { break };
            let ord = ca.to_lowercase().cmp(cb.to_lowercase());
            if ord != Ordering::Equal {
                return ord;
            }
            i += ca.len_utf8();
            j += cb.len_utf8();
        }
    }
    match (i < x.len(), j < y.len()) {
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
    }
}

fn trim_zeros(digits: &[u8]) -> &[u8] {
    let start = digits.iter().position(|d| *d != b'0').unwrap_or(digits.len());
    &digits[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec!["file10", "File2", "file1", "file02", "a"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["a", "file1", "file02", "File2", "file10"]);
    }

    #[test]
    fn order_is_total_and_consistent() {
        let names = ["b", "B", "a1", "a01", "a001", "ä", "a", "A", "10", "9"];
        for x in names {
            for y in names {
                assert_eq!(natural_cmp(x, y), natural_cmp(y, x).reverse(), "{x} vs {y}");
                assert_eq!(natural_cmp(x, y) == Ordering::Equal, x == y, "{x} vs {y}");
            }
        }
    }
}

/// What a listing is sorted by. Names break ties in every mode, so the order is total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Kind,
    Modified,
    Added,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub ascending: bool,
    pub folders_first: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self { key: SortKey::Name, ascending: true, folders_first: true }
    }
}

impl Sort {
    /// Clicking a column header: the same column flips direction; a new one starts
    /// in Finder's direction for it (dates and sizes newest/largest first).
    pub fn toggled(self, key: SortKey) -> Sort {
        if key == self.key {
            Sort { ascending: !self.ascending, ..self }
        } else {
            let ascending = matches!(key, SortKey::Name | SortKey::Kind);
            Sort { key, ascending, ..self }
        }
    }
}

/// Compare two entries under `sort`. `kind` supplies kind names (only used for `SortKey::Kind`).
pub fn compare(
    a: &crate::Entry,
    b: &crate::Entry,
    sort: &Sort,
    kind: &(dyn Fn(&crate::Entry) -> std::sync::Arc<str> + Sync),
) -> Ordering {
    if sort.folders_first && a.is_dir != b.is_dir {
        return b.is_dir.cmp(&a.is_dir);
    }
    let primary = match sort.key {
        SortKey::Name => Ordering::Equal,
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Kind => natural_cmp(&kind(a), &kind(b)),
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Added => a.added.cmp(&b.added),
    };
    let by_name = natural_cmp(&a.name, &b.name);
    match (sort.key, sort.ascending) {
        (SortKey::Name, true) => by_name,
        (SortKey::Name, false) => by_name.reverse(),
        (_, true) => primary.then(by_name),
        (_, false) => primary.reverse().then(by_name),
    }
}

/// Sort a listing in place. Kind names are looked up once per entry, not per comparison.
pub fn sort_entries(
    entries: &mut Vec<crate::Entry>,
    sort: &Sort,
    kind: &(dyn Fn(&crate::Entry) -> std::sync::Arc<str> + Sync),
) {
    use rayon::slice::ParallelSliceMut;
    if sort.key != SortKey::Kind {
        entries.par_sort_unstable_by(|a, b| compare(a, b, sort, kind));
        return;
    }
    let mut keyed: Vec<(std::sync::Arc<str>, crate::Entry)> = entries.drain(..).map(|e| (kind(&e), e)).collect();
    keyed.par_sort_unstable_by(|(ka, a), (kb, b)| {
        let cached = |e: &crate::Entry| if std::ptr::eq(e, a) { ka.clone() } else { kb.clone() };
        compare(a, b, sort, &cached)
    });
    entries.extend(keyed.into_iter().map(|(_, e)| e));
}

#[cfg(test)]
mod sort_tests {
    use super::*;
    use crate::Entry;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn entry(name: &str, size: u64, secs: u64, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: format!("/x/{name}").into(),
            is_dir,
            is_symlink: false,
            icon_kind: crate::IconKind::default(),
            hidden: false,
            dataless: false,
            size,
            modified: Some(UNIX_EPOCH + Duration::from_secs(secs)),
            created: None,
            added: Some(UNIX_EPOCH + Duration::from_secs(1000 - secs)),
        }
    }

    fn kind(e: &Entry) -> Arc<str> {
        Arc::from(if e.is_dir { "Folder" } else { e.ext() })
    }

    fn sorted(sort: Sort) -> Vec<String> {
        let mut v = vec![
            entry("b.txt", 30, 3, false),
            entry("a.md", 10, 1, false),
            entry("dir", 0, 2, true),
            entry("c.md", 20, 5, false),
        ];
        sort_entries(&mut v, &sort, &kind);
        v.into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn each_key_and_direction() {
        let base = Sort::default();
        assert_eq!(sorted(base), ["dir", "a.md", "b.txt", "c.md"]);
        assert_eq!(sorted(Sort { ascending: false, ..base }), ["dir", "c.md", "b.txt", "a.md"]);
        assert_eq!(sorted(base.toggled(SortKey::Size)), ["dir", "b.txt", "c.md", "a.md"]);
        assert_eq!(sorted(base.toggled(SortKey::Modified)), ["dir", "c.md", "b.txt", "a.md"]);
        assert_eq!(sorted(base.toggled(SortKey::Added)), ["dir", "a.md", "b.txt", "c.md"]);
        assert_eq!(sorted(base.toggled(SortKey::Kind)), ["dir", "a.md", "c.md", "b.txt"]);
        assert_eq!(
            sorted(Sort { folders_first: false, ..base.toggled(SortKey::Kind) }),
            ["dir", "a.md", "c.md", "b.txt"]
        );
    }

    #[test]
    fn toggling_the_same_column_flips_it() {
        let s = Sort::default().toggled(SortKey::Size);
        assert!(!s.ascending, "sizes start largest first");
        assert!(s.toggled(SortKey::Size).ascending);
    }
}

#[cfg(test)]
mod natural_tests {
    use super::*;
    use std::iter::Peekable;
    use std::str::Chars;

    /// The previous, allocating implementation: simple enough to trust as the reference.
    fn reference(a: &str, b: &str) -> Ordering {
        fn take_digits(chars: &mut Peekable<Chars>) -> String {
            let mut digits = String::new();
            while let Some(&c) = chars.peek() {
                if !c.is_ascii_digit() {
                    break;
                }
                digits.push(c);
                chars.next();
            }
            digits
        }
        let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
        loop {
            match (left.peek().copied(), right.peek().copied()) {
                (None, None) => return a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                    let (n, m) = (take_digits(&mut left), take_digits(&mut right));
                    let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                    let ord = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                (Some(x), Some(y)) => {
                    let ord = x.to_lowercase().cmp(y.to_lowercase());
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    left.next();
                    right.next();
                }
            }
        }
    }

    #[test]
    fn agrees_with_reference_on_tricky_names() {
        let names = [
            "file2",
            "file10",
            "File2",
            "file02",
            "file",
            "files",
            "a",
            "A",
            "ä",
            "Ä",
            "z",
            "10",
            "9",
            "09",
            "image 1.png",
            "image 1 copy.png",
            "image 10.png",
            "IMG_0001.HEIC",
            "img_1.heic",
            "Ωmega",
            "émile",
            "e",
            "x1y2",
            "x01y10",
            "",
            "名前",
            "名前2",
            "a\u{0301}",
            "ß",
            "SS",
            "İstanbul",
            "0",
            "00",
            "007",
        ];
        for a in names {
            for b in names {
                assert_eq!(natural_cmp(a, b), reference(a, b), "{a:?} vs {b:?}");
            }
        }
    }
}
