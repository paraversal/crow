//! Listing a folder. On macOS this uses `getattrlistbulk(2)`, which returns
//! names, types, sizes, dates and flags for many entries per system call
//! (what Finder uses) instead of one `lstat` per file.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Starts with a dot, or carries the Finder "hidden" flag (like ~/Library).
    pub hidden: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    /// When the item was put in this folder (Finder's "Date Added").
    pub added: Option<SystemTime>,
}

impl Entry {
    pub fn ext(&self) -> &str {
        Path::new(&self.name).extension().and_then(|e| e.to_str()).unwrap_or("")
    }

    /// Application bundles and other packages open rather than navigate.
    pub fn is_package(&self) -> bool {
        self.is_dir && matches!(self.ext(), "app" | "bundle" | "framework" | "photoslibrary")
    }
}

/// List `dir`, calling `on_batch` with entries as they are read (unsorted).
/// Unreadable entries are skipped rather than failing the whole folder.
pub fn list_dir(dir: &Path, show_hidden: bool, mut on_batch: impl FnMut(Vec<Entry>)) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        match bulk::list(dir, show_hidden, &mut on_batch) {
            Err(e) if e.raw_os_error() == Some(libc::ENOTSUP) => {}
            other => return other,
        }
    }
    portable::list(dir, show_hidden, &mut on_batch)
}

/// Read one item, for applying a single change without relisting its folder.
/// `Ok(None)`: it no longer exists.
pub fn stat_entry(path: &Path) -> std::io::Result<Option<Entry>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let is_dir = meta.is_dir() || (meta.file_type().is_symlink() && path.is_dir());
    #[cfg(target_os = "macos")]
    let (hidden_flag, added) = bulk::flags_and_added(path);
    #[cfg(not(target_os = "macos"))]
    let (hidden_flag, added) = (false, None);
    Ok(Some(Entry {
        hidden: name.starts_with('.') || hidden_flag,
        path: path.to_path_buf(),
        name,
        is_dir,
        size: if is_dir { 0 } else { meta.len() },
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        added,
    }))
}

mod portable {
    use super::Entry;
    use std::path::Path;

    pub fn list(dir: &Path, show_hidden: bool, on_batch: &mut dyn FnMut(Vec<Entry>)) -> std::io::Result<()> {
        let mut batch = Vec::with_capacity(1024);
        for dirent in std::fs::read_dir(dir)?.filter_map(Result::ok) {
            let Ok(Some(entry)) = super::stat_entry(&dirent.path()) else { continue };
            if entry.hidden && !show_hidden {
                continue;
            }
            batch.push(entry);
            if batch.len() == 1024 {
                on_batch(std::mem::take(&mut batch));
            }
        }
        on_batch(batch);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod bulk {
    use super::Entry;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    // Attribute bits, in the order the kernel lays them out (ascending bit value,
    // except RETURNED_ATTRS first and ERROR right after it).
    const COMMON: u32 = libc::ATTR_CMN_RETURNED_ATTRS
        | ATTR_CMN_ERROR
        | libc::ATTR_CMN_NAME
        | libc::ATTR_CMN_OBJTYPE
        | libc::ATTR_CMN_CRTIME
        | libc::ATTR_CMN_MODTIME
        | libc::ATTR_CMN_FLAGS
        | libc::ATTR_CMN_ADDEDTIME;
    /// From <sys/attr.h>; not exported by the libc crate.
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const FILE: u32 = libc::ATTR_FILE_TOTALSIZE;
    const BUFFER: usize = 256 * 1024;
    const VDIR: u32 = 2;
    const VLNK: u32 = 5;
    const UF_HIDDEN: u32 = 0x8000;

    fn attr_list(common: u32, file: u32) -> libc::attrlist {
        libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: common,
            volattr: 0,
            dirattr: 0,
            fileattr: file,
            forkattr: 0,
        }
    }

    /// Reads little values out of the kernel's packed buffer. Every read is
    /// bounds-checked against the entry, so a malformed entry is skipped, never read past.
    struct Cursor<'a> {
        entry: &'a [u8],
        at: usize,
    }

    impl<'a> Cursor<'a> {
        fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
            let bytes = self.entry.get(self.at..self.at + N)?.try_into().ok()?;
            self.at += N;
            Some(bytes)
        }
        fn u32(&mut self) -> Option<u32> {
            self.take::<4>().map(u32::from_ne_bytes)
        }
        fn i64(&mut self) -> Option<i64> {
            self.take::<8>().map(i64::from_ne_bytes)
        }
        fn time(&mut self) -> Option<SystemTime> {
            let (secs, nanos) = (self.i64()?, self.i64()?);
            let since = Duration::new(secs.unsigned_abs(), nanos.clamp(0, 999_999_999) as u32);
            Some(if secs >= 0 { UNIX_EPOCH + since } else { UNIX_EPOCH - since })
        }
        /// An `attrreference_t`: offset (relative to the reference itself) and length.
        fn name(&mut self) -> Option<String> {
            let start = self.at;
            let offset = self.take::<4>().map(i32::from_ne_bytes)?;
            let length = self.u32()? as usize;
            let begin = usize::try_from(start as i64 + i64::from(offset)).ok()?;
            let raw = self.entry.get(begin..begin + length)?;
            let raw = raw.split(|b| *b == 0).next()?;
            Some(String::from_utf8_lossy(raw).into_owned())
        }
    }

    fn parse(entry: &[u8], dir: &Path) -> Option<Entry> {
        let mut c = Cursor { entry, at: 4 };
        let returned = [c.u32()?, c.u32()?, c.u32()?, c.u32()?, c.u32()?];
        let (common, file) = (returned[0], returned[3]);
        if common & ATTR_CMN_ERROR != 0 && c.u32()? != 0 {
            return None;
        }
        let name = if common & libc::ATTR_CMN_NAME != 0 { c.name()? } else { return None };
        let objtype = if common & libc::ATTR_CMN_OBJTYPE != 0 { c.u32()? } else { 0 };
        let created = if common & libc::ATTR_CMN_CRTIME != 0 { c.time() } else { None };
        let modified = if common & libc::ATTR_CMN_MODTIME != 0 { c.time() } else { None };
        let flags = if common & libc::ATTR_CMN_FLAGS != 0 { c.u32()? } else { 0 };
        let added = if common & libc::ATTR_CMN_ADDEDTIME != 0 { c.time() } else { None };
        let size = if file & libc::ATTR_FILE_TOTALSIZE != 0 { c.i64().map_or(0, |s| s.max(0) as u64) } else { 0 };
        let path = dir.join(&name);
        // Links are listed as links; only those need a second look to learn if they point at a folder.
        let is_dir = objtype == VDIR || (objtype == VLNK && path.is_dir());
        Some(Entry {
            hidden: name.starts_with('.') || flags & UF_HIDDEN != 0,
            name,
            path,
            is_dir,
            size: if is_dir { 0 } else { size },
            modified,
            created,
            added,
        })
    }

    pub fn list(dir: &Path, show_hidden: bool, on_batch: &mut dyn FnMut(Vec<Entry>)) -> std::io::Result<()> {
        let c_dir = CString::new(dir.as_os_str().as_bytes())?;
        // SAFETY: c_dir is a valid NUL-terminated path; the fd is closed below on every path.
        let fd = unsafe { libc::open(c_dir.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = read_all(fd, dir, show_hidden, on_batch);
        // SAFETY: fd is ours and still open.
        unsafe { libc::close(fd) };
        result
    }

    fn read_all(fd: i32, dir: &Path, show_hidden: bool, on_batch: &mut dyn FnMut(Vec<Entry>)) -> std::io::Result<()> {
        let mut attrs = attr_list(COMMON, FILE);
        let mut buffer = vec![0u8; BUFFER];
        loop {
            // SAFETY: attrs and buffer are valid for the call and buffer.len() is its size.
            let count = unsafe {
                libc::getattrlistbulk(fd, (&raw mut attrs).cast(), buffer.as_mut_ptr().cast(), buffer.len(), 0)
            };
            if count < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(());
            }
            let mut batch = Vec::with_capacity(count as usize);
            let mut at = 0usize;
            for _ in 0..count {
                let Some(len) = buffer.get(at..at + 4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as usize)
                else {
                    break;
                };
                let Some(entry) = buffer.get(at..at + len) else { break };
                if let Some(e) = parse(entry, dir).filter(|e| show_hidden || !e.hidden) {
                    batch.push(e);
                }
                if len == 0 {
                    break;
                }
                at += len;
            }
            on_batch(batch);
        }
    }

    /// The Finder hidden flag and Date Added for one path, for single-item updates.
    pub fn flags_and_added(path: &Path) -> (bool, Option<SystemTime>) {
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else { return (false, None) };
        let mut attrs = attr_list(libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_FLAGS | libc::ATTR_CMN_ADDEDTIME, 0);
        let mut buffer = [0u8; 128];
        // SAFETY: valid path, attrs and buffer; FSOPT_NOFOLLOW reads the link itself.
        let rc = unsafe {
            libc::getattrlist(
                c_path.as_ptr(),
                (&raw mut attrs).cast(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::FSOPT_NOFOLLOW,
            )
        };
        if rc != 0 {
            return (false, None);
        }
        let mut c = Cursor { entry: &buffer, at: 4 };
        let Some(common) = c.u32() else { return (false, None) };
        c.at += 16; // the other four groups of attribute_set_t
        let flags = if common & libc::ATTR_CMN_FLAGS != 0 { c.u32().unwrap_or(0) } else { 0 };
        let added = if common & libc::ATTR_CMN_ADDEDTIME != 0 { c.time() } else { None };
        (flags & UF_HIDDEN != 0, added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crow-fs-list-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.canonicalize().expect("canonical")
    }

    fn collect(dir: &Path, hidden: bool) -> BTreeMap<String, Entry> {
        let mut all = BTreeMap::new();
        list_dir(dir, hidden, |batch| all.extend(batch.into_iter().map(|e| (e.name.clone(), e)))).expect("list");
        all
    }

    #[test]
    fn bulk_listing_matches_per_file_stat() {
        let dir = temp_dir("match");
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::write(dir.join("ünïcødé 名前.md"), "x".repeat(5000)).unwrap();
        std::fs::write(dir.join(".dot"), "").unwrap();
        std::fs::create_dir(dir.join("folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("folder"), dir.join("link-to-folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), dir.join("dangling")).unwrap();

        let listed = collect(&dir, true);
        assert_eq!(listed.len(), 6);
        for (name, entry) in &listed {
            let expected = stat_entry(&dir.join(name)).unwrap().unwrap();
            assert_eq!(entry.is_dir, expected.is_dir, "{name}");
            assert_eq!(entry.size, expected.size, "{name}");
            assert_eq!(entry.modified, expected.modified, "{name}");
            assert_eq!(entry.hidden, expected.hidden, "{name}");
        }
        assert!(listed["link-to-folder"].is_dir);
        assert!(!listed["dangling"].is_dir);
        assert_eq!(listed["ünïcødé 名前.md"].size, 5000);
        assert!(!collect(&dir, false).contains_key(".dot"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn large_folder_arrives_in_several_batches() {
        let dir = temp_dir("batches");
        crate::make_fixture(&dir, 5000).unwrap();
        let (mut batches, mut total) = (0, 0);
        list_dir(&dir, false, |b| {
            batches += 1;
            total += b.len();
        })
        .unwrap();
        assert_eq!(total, 5000);
        assert!(batches >= 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_folder_is_an_error() {
        assert!(list_dir(Path::new("/definitely/not/here"), false, |_| {}).is_err());
    }
}
