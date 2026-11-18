//! Host-side bounded excerpt reads. The Rust host calls this before enqueueing
//! a worker request; the script only ever receives the resulting text.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use crate::dto::EntryRef;

/// Read at most `max_bytes` of UTF-8 text from an approved entry. Rejects
/// symlinks, non-files and an entry whose identity changed since the request
/// was built.
pub fn read_excerpt(entry: &EntryRef, max_bytes: usize) -> Result<String, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&entry.path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) => format!("{}: symbolic links are not allowed", entry.path.display()),
            _ => format!("{}: {error}", entry.path.display()),
        })?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", entry.path.display()));
    }
    if let Some((device, inode)) = entry.identity
        && (metadata.dev() != device || metadata.ino() != inode)
    {
        return Err("entry identity changed since the request was created".into());
    }
    let mut bytes = Vec::with_capacity(max_bytes.min(metadata.len() as usize));
    file.take(max_bytes as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    bounded_excerpt(bytes, max_bytes)
}

/// Cut a bounded excerpt without splitting a UTF-8 character at the cap. A
/// newline is the preferred cut; otherwise only an incomplete trailing
/// sequence is dropped. Genuinely invalid bytes still fail the read.
fn bounded_excerpt(mut bytes: Vec<u8>, max_bytes: usize) -> Result<String, String> {
    if bytes.len() > max_bytes {
        bytes.truncate(max_bytes);
        if let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') {
            bytes.truncate(last_newline + 1);
        } else if let Err(error) = std::str::from_utf8(&bytes)
            && error.error_len().is_none()
        {
            bytes.truncate(error.valid_up_to());
        }
    }
    if bytes.contains(&0) {
        return Err("file is binary".into());
    }
    String::from_utf8(bytes).map_err(|_| "file is not UTF-8 text".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn reads_a_bounded_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "hello").unwrap();
        assert_eq!(read_excerpt(&EntryRef::new(path), 64 * 1024).unwrap(), "hello");
    }

    #[test]
    fn rejects_a_symlink_and_a_changed_identity() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&target, "hello").unwrap();
        symlink(&target, &link).unwrap();
        assert!(read_excerpt(&EntryRef::new(&link), 64 * 1024).unwrap_err().contains("symbolic link"));

        let metadata = std::fs::metadata(&target).unwrap();
        let changed = EntryRef::with_identity(target, metadata.dev(), metadata.ino() + 1);
        assert!(read_excerpt(&changed, 64 * 1024).unwrap_err().contains("identity changed"));
    }

    #[test]
    fn excerpt_cap_does_not_split_a_utf8_character() {
        let prefix = "a".repeat(64 * 1024 - 1);
        let mut bytes = prefix.clone().into_bytes();
        bytes.extend_from_slice("é".as_bytes());
        assert_eq!(bounded_excerpt(bytes, 64 * 1024).unwrap(), prefix);
    }

    #[test]
    fn excerpt_cap_still_rejects_invalid_utf8_inside_the_bound() {
        let mut bytes = vec![b'a'; 99];
        bytes.push(0xff);
        bytes.push(b'b');
        assert!(bounded_excerpt(bytes, 100).is_err());
    }
}
