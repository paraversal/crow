use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// Case-insensitive natural order (`file2` before `file10`). Names equal under
/// natural order fall back to case-insensitive, then byte order, so the order is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
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
