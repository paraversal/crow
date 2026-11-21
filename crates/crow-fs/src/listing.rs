//! Listing a folder. On macOS this uses `getattrlistbulk(2)`, which returns
//! names, types, sizes, dates and flags for many entries per system call
//! (what Finder uses) instead of one `lstat` per file.

#[cfg(target_os = "macos")]
use std::os::macos::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Whether an item's native icon can be shared by type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconKind {
    File,
    Folder,
    Executable,
    /// A symlink, alias, custom icon, package, or item with unknown metadata.
    #[default]
    Path,
}

impl IconKind {
    fn classify(path: &Path, is_dir: bool, symlink: bool, mode: Option<u32>, finder_flags: u16) -> Self {
        const HAS_CUSTOM_ICON: u16 = 0x0400;
        const IS_ALIAS: u16 = 0x8000;
        if symlink || finder_flags & (HAS_CUSTOM_ICON | IS_ALIAS) != 0 {
            return Self::Path;
        }
        if is_dir {
            // Preserve Finder's special folder icons, including packages.
            return if path.extension().is_some() { Self::Path } else { Self::Folder };
        }
        match mode {
            Some(mode) if mode & 0o111 != 0 => {
                if path.extension().is_some() {
                    Self::Path
                } else {
                    Self::Executable
                }
            }
            Some(_) => Self::File,
            None => Self::Path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// The entry itself is a symbolic link (its target may be a folder).
    pub is_symlink: bool,
    /// Starts with a dot, or carries the Finder "hidden" flag (like ~/Library).
    pub hidden: bool,
    /// The file is a cloud placeholder (`SF_DATALESS`); reading it would hydrate.
    pub dataless: bool,
    pub icon_kind: IconKind,
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
        self.is_dir && is_package_name(&self.name)
    }
}

/// Application bundles and other packages open rather than navigate.
pub fn is_package_name(name: &str) -> bool {
    matches!(
        Path::new(name).extension().and_then(|e| e.to_str()),
        Some("app" | "bundle" | "framework" | "photoslibrary")
    )
}

/// The same test for a path, for the size walk.
pub fn is_package_path(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(is_package_name)
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

/// `list_dir`'s bounded sibling: why a sample is not the whole folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleIncomplete {
    /// The visible-entry cap was reached; the folder has more children.
    LimitReached { examined: usize },
    /// The scan guard was reached before the visible-entry cap.
    ExaminedLimit { examined: usize },
    /// A cancellation flag was set between entries or batches.
    Cancelled { examined: usize },
}

/// A bounded metadata sample of one folder's immediate children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderSample {
    /// At most the requested number of visible entries, in raw directory order.
    pub entries: Vec<Entry>,
    /// Raw dirents examined, including hidden ones.
    pub examined: usize,
    /// Hidden raw dirents among `examined`.
    pub hidden_examined: usize,
    /// `None` means the folder was read to EOF.
    pub incomplete: Option<SampleIncomplete>,
}

/// List at most `max_entries` visible children, examining at most `max_examined`
/// raw dirents. Unlike `list_dir`, this stops the directory read once either cap
/// is reached, so previewing a huge folder does not walk all of it.
pub fn list_sample(
    dir: &Path,
    show_hidden: bool,
    max_entries: usize,
    max_examined: Option<usize>,
    cancel: &crate::sizes::Cancel,
) -> std::io::Result<FolderSample> {
    #[cfg(target_os = "macos")]
    {
        match bulk::sample(dir, show_hidden, max_entries, max_examined, cancel) {
            Err(e) if e.raw_os_error() == Some(libc::ENOTSUP) => {}
            other => return other,
        }
    }
    portable::sample(dir, show_hidden, max_entries, max_examined, cancel)
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
    let (hidden_flag, added, finder_flags) = bulk::item_attributes(path);
    #[cfg(not(target_os = "macos"))]
    let (hidden_flag, added, finder_flags) = (false, None, 0);
    #[cfg(unix)]
    let mode = Some(std::os::unix::fs::MetadataExt::mode(&meta));
    #[cfg(not(unix))]
    let mode = Some(0);
    #[cfg(target_os = "macos")]
    let dataless = crate::cloud::dataless_flags(meta.st_flags());
    #[cfg(not(target_os = "macos"))]
    let dataless = false;
    Ok(Some(Entry {
        hidden: name.starts_with('.') || hidden_flag,
        path: path.to_path_buf(),
        name,
        is_dir,
        is_symlink: meta.file_type().is_symlink(),
        dataless,
        icon_kind: IconKind::classify(path, is_dir, meta.file_type().is_symlink(), mode, finder_flags),
        size: if is_dir { 0 } else { meta.len() },
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        added,
    }))
}

mod portable {
    use super::{Entry, FolderSample, SampleIncomplete};
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

    pub fn sample(
        dir: &Path,
        show_hidden: bool,
        max_entries: usize,
        max_examined: Option<usize>,
        cancel: &crate::sizes::Cancel,
    ) -> std::io::Result<FolderSample> {
        let mut sample = FolderSample { entries: Vec::new(), examined: 0, hidden_examined: 0, incomplete: None };
        for dirent in std::fs::read_dir(dir)?.filter_map(Result::ok) {
            if cancel.is_cancelled() {
                sample.incomplete = Some(SampleIncomplete::Cancelled { examined: sample.examined });
                return Ok(sample);
            }
            if max_examined.is_some_and(|max| sample.examined >= max) {
                sample.incomplete = Some(SampleIncomplete::ExaminedLimit { examined: sample.examined });
                return Ok(sample);
            }
            sample.examined += 1;
            let Ok(Some(entry)) = super::stat_entry(&dirent.path()) else { continue };
            if entry.hidden {
                sample.hidden_examined += 1;
                if !show_hidden {
                    continue;
                }
            }
            if sample.entries.len() >= max_entries {
                sample.incomplete = Some(SampleIncomplete::LimitReached { examined: sample.examined });
                return Ok(sample);
            }
            sample.entries.push(entry);
        }
        Ok(sample)
    }
}

#[cfg(target_os = "macos")]
pub(crate) use bulk::list_raw;

#[cfg(target_os = "macos")]
mod bulk {
    use super::{Entry, FolderSample, IconKind, SampleIncomplete};
    use std::ffi::CString;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    // Attribute bits, in the order the kernel lays them out (ascending bit value,
    // except RETURNED_ATTRS first and ERROR right after it).
    fn common(times: bool) -> u32 {
        let mut common = libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_DEVID
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_FLAGS
            | libc::ATTR_CMN_FILEID;
        if times {
            common |= libc::ATTR_CMN_CRTIME
                | libc::ATTR_CMN_MODTIME
                | libc::ATTR_CMN_FNDRINFO
                | libc::ATTR_CMN_ACCESSMASK
                | libc::ATTR_CMN_ADDEDTIME;
        }
        common
    }
    /// From <sys/attr.h>; not exported by the libc crate.
    const ATTR_CMN_ERROR: u32 = 0x2000_0000;
    const FILE: u32 = libc::ATTR_FILE_TOTALSIZE | libc::ATTR_FILE_LINKCOUNT;
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
        fn u64(&mut self) -> Option<u64> {
            self.take::<8>().map(u64::from_ne_bytes)
        }
        fn i64(&mut self) -> Option<i64> {
            self.take::<8>().map(i64::from_ne_bytes)
        }
        fn time(&mut self) -> Option<SystemTime> {
            let (secs, nanos) = (self.i64()?, self.i64()?);
            let since = Duration::new(secs.unsigned_abs(), nanos.clamp(0, 999_999_999) as u32);
            Some(if secs >= 0 { UNIX_EPOCH + since } else { UNIX_EPOCH - since })
        }
        fn finder_flags(&mut self) -> Option<u16> {
            let finder_info = self.take::<32>()?;
            // FinderInfo stores its flags in big-endian order at offset eight.
            Some(u16::from_be_bytes([finder_info[8], finder_info[9]]))
        }

        /// An `attrreference_t`: offset (relative to the reference itself) and length.
        /// The bytes are kept exactly; macOS does not require UTF-8 file names.
        fn name(&mut self) -> Option<OsString> {
            use std::os::unix::ffi::OsStringExt;
            let start = self.at;
            let offset = self.take::<4>().map(i32::from_ne_bytes)?;
            let length = self.u32()? as usize;
            let begin = usize::try_from(start as i64 + i64::from(offset)).ok()?;
            let raw = self.entry.get(begin..begin + length)?;
            let raw = raw.split(|b| *b == 0).next()?;
            Some(OsString::from_vec(raw.to_vec()))
        }
    }

    /// One directory entry as the kernel returned it, before the listing turns it
    /// into an `Entry`. The sizer uses the same walk without the listing's fields.
    pub(crate) struct RawEntry<'a> {
        pub name: OsString,
        dir: &'a Path,
        pub objtype: u32,
        pub dev: u64,
        pub fileid: u64,
        pub nlink: u32,
        pub size: u64,
        pub flags: u32,
        finder_flags: u16,
        mode: Option<u32>,
        pub created: Option<SystemTime>,
        pub modified: Option<SystemTime>,
        pub added: Option<SystemTime>,
    }

    impl RawEntry<'_> {
        pub fn path(&self) -> PathBuf {
            self.dir.join(&self.name)
        }

        pub fn hidden(&self) -> bool {
            use std::os::unix::ffi::OsStrExt;
            self.name.as_os_str().as_bytes().starts_with(b".") || self.flags & UF_HIDDEN != 0
        }

        /// Links are listed as links; only those need a second look to learn if they point at a folder.
        pub fn is_dir(&self) -> bool {
            self.objtype == VDIR || (self.objtype == VLNK && self.path().is_dir())
        }

        pub fn is_symlink(&self) -> bool {
            self.objtype == VLNK
        }

        fn into_entry(self, show_hidden: bool) -> Option<Entry> {
            let hidden = self.hidden();
            if hidden && !show_hidden {
                return None;
            }
            let is_dir = self.is_dir();
            let is_symlink = self.is_symlink();
            let path = self.path();
            Some(Entry {
                icon_kind: IconKind::classify(&path, is_dir, is_symlink, self.mode, self.finder_flags),
                hidden,
                name: self.name.to_string_lossy().into_owned(),
                path,
                is_dir,
                is_symlink,
                dataless: crate::cloud::dataless_flags(self.flags),
                size: if is_dir { 0 } else { self.size },
                modified: self.modified,
                created: self.created,
                added: self.added,
            })
        }
    }

    fn parse<'a>(entry: &[u8], dir: &'a Path) -> Option<RawEntry<'a>> {
        let mut c = Cursor { entry, at: 4 };
        let returned = [c.u32()?, c.u32()?, c.u32()?, c.u32()?, c.u32()?];
        let (common, file) = (returned[0], returned[3]);
        if common & ATTR_CMN_ERROR != 0 && c.u32()? != 0 {
            return None;
        }
        let name = if common & libc::ATTR_CMN_NAME != 0 { c.name()? } else { return None };
        let dev = if common & libc::ATTR_CMN_DEVID != 0 { c.u32()? as u64 } else { 0 };
        let objtype = if common & libc::ATTR_CMN_OBJTYPE != 0 { c.u32()? } else { 0 };
        let created = if common & libc::ATTR_CMN_CRTIME != 0 { c.time() } else { None };
        let modified = if common & libc::ATTR_CMN_MODTIME != 0 { c.time() } else { None };
        let finder_flags = if common & libc::ATTR_CMN_FNDRINFO != 0 { c.finder_flags()? } else { 0 };
        let mode = if common & libc::ATTR_CMN_ACCESSMASK != 0 { Some(c.u32()?) } else { None };
        let flags = if common & libc::ATTR_CMN_FLAGS != 0 { c.u32()? } else { 0 };
        let fileid = if common & libc::ATTR_CMN_FILEID != 0 { c.u64()? } else { 0 };
        let added = if common & libc::ATTR_CMN_ADDEDTIME != 0 { c.time() } else { None };
        let nlink = if file & libc::ATTR_FILE_LINKCOUNT != 0 { c.u32()? } else { 1 };
        let size = if file & libc::ATTR_FILE_TOTALSIZE != 0 { c.i64().map_or(0, |s| s.max(0) as u64) } else { 0 };
        Some(RawEntry {
            name,
            dir,
            objtype,
            dev,
            fileid,
            nlink,
            size,
            flags,
            finder_flags,
            mode,
            created,
            modified,
            added,
        })
    }

    pub fn list(dir: &Path, show_hidden: bool, on_batch: &mut dyn FnMut(Vec<Entry>)) -> std::io::Result<()> {
        list_raw(dir, true, &mut |raw| {
            let batch: Vec<Entry> = raw.into_iter().filter_map(|entry| entry.into_entry(show_hidden)).collect();
            if !batch.is_empty() {
                on_batch(batch);
            }
            true
        })
    }

    pub fn sample(
        dir: &Path,
        show_hidden: bool,
        max_entries: usize,
        max_examined: Option<usize>,
        cancel: &crate::sizes::Cancel,
    ) -> std::io::Result<FolderSample> {
        let mut sample = FolderSample { entries: Vec::new(), examined: 0, hidden_examined: 0, incomplete: None };
        list_raw(dir, true, &mut |raw| {
            for entry in raw {
                if cancel.is_cancelled() {
                    sample.incomplete = Some(SampleIncomplete::Cancelled { examined: sample.examined });
                    return false;
                }
                if max_examined.is_some_and(|max| sample.examined >= max) {
                    sample.incomplete = Some(SampleIncomplete::ExaminedLimit { examined: sample.examined });
                    return false;
                }
                sample.examined += 1;
                if entry.hidden() {
                    sample.hidden_examined += 1;
                    if !show_hidden {
                        continue;
                    }
                }
                if sample.entries.len() >= max_entries {
                    sample.incomplete = Some(SampleIncomplete::LimitReached { examined: sample.examined });
                    return false;
                }
                if let Some(entry) = entry.into_entry(true) {
                    sample.entries.push(entry);
                }
            }
            true
        })?;
        Ok(sample)
    }

    pub(crate) fn list_raw<'a>(
        dir: &'a Path,
        times: bool,
        on_batch: &mut dyn FnMut(Vec<RawEntry<'a>>) -> bool,
    ) -> std::io::Result<()> {
        let c_dir = CString::new(dir.as_os_str().as_bytes())?;
        // SAFETY: c_dir is a valid NUL-terminated path; the fd is closed below on every path.
        let fd = unsafe { libc::open(c_dir.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = read_all(fd, dir, times, on_batch);
        // SAFETY: fd is ours and still open.
        unsafe { libc::close(fd) };
        result
    }

    fn read_all<'a>(
        fd: i32,
        dir: &'a Path,
        times: bool,
        on_batch: &mut dyn FnMut(Vec<RawEntry<'a>>) -> bool,
    ) -> std::io::Result<()> {
        let mut attrs = attr_list(common(times), FILE);
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
                if let Some(e) = parse(entry, dir) {
                    batch.push(e);
                }
                if len == 0 {
                    break;
                }
                at += len;
            }
            if !on_batch(batch) {
                return Ok(());
            }
        }
    }

    /// Finder flags and Date Added for a single-item listing update.
    pub fn item_attributes(path: &Path) -> (bool, Option<SystemTime>, u16) {
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else { return (false, None, 0) };
        let mut attrs = attr_list(
            libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_FNDRINFO | libc::ATTR_CMN_FLAGS | libc::ATTR_CMN_ADDEDTIME,
            0,
        );
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
            return (false, None, 0);
        }
        let mut c = Cursor { entry: &buffer, at: 4 };
        let Some(common) = c.u32() else { return (false, None, 0) };
        c.at += 16; // the other four groups of attribute_set_t
        let finder_flags = if common & libc::ATTR_CMN_FNDRINFO != 0 { c.finder_flags().unwrap_or(0) } else { 0 };
        let flags = if common & libc::ATTR_CMN_FLAGS != 0 { c.u32().unwrap_or(0) } else { 0 };
        let added = if common & libc::ATTR_CMN_ADDEDTIME != 0 { c.time() } else { None };
        (flags & UF_HIDDEN != 0, added, finder_flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp_dir(name: &str) -> crate::test_support::TempDir {
        crate::test_support::TempDir::new(&format!("crow-fs-list-{name}-"))
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
            assert_eq!(entry.icon_kind, expected.icon_kind, "{name}");
        }
        assert!(listed["link-to-folder"].is_dir);
        assert!(!listed["dangling"].is_dir);
        assert_eq!(listed["ünïcødé 名前.md"].size, 5000);
        assert!(!collect(&dir, false).contains_key(".dot"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_icon_kinds_agree_between_bulk_and_single_item_updates() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("icons");
        for name in [".localized", "run", "alias.txt", "custom.txt"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        std::fs::set_permissions(dir.join("run"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir(dir.join("Test.app")).unwrap();

        for (name, flags) in [("alias.txt", 0x8000u16), ("custom.txt", 0x0400u16)] {
            let path = std::ffi::CString::new(dir.join(name).as_os_str().as_bytes()).unwrap();
            let mut finder_info = [0u8; 32];
            finder_info[8..10].copy_from_slice(&flags.to_be_bytes());
            // SAFETY: both C strings and the attribute bytes live through the call.
            let result = unsafe {
                libc::setxattr(path.as_ptr(), c"com.apple.FinderInfo".as_ptr(), finder_info.as_ptr().cast(), 32, 0, 0)
            };
            assert_eq!(result, 0, "{name}: {}", std::io::Error::last_os_error());
        }

        let listed = collect(&dir, true);
        for (name, expected) in [
            (".localized", IconKind::File),
            ("run", IconKind::Executable),
            ("alias.txt", IconKind::Path),
            ("custom.txt", IconKind::Path),
            ("Test.app", IconKind::Path),
        ] {
            assert_eq!(listed[name].icon_kind, expected, "{name} from bulk listing");
            assert_eq!(stat_entry(&dir.join(name)).unwrap().unwrap().icon_kind, expected, "{name} from stat");
        }
    }

    #[test]
    fn large_folder_arrives_in_several_batches() {
        let dir = temp_dir("batches");
        crate::testing::make_wide(&dir, 5000).unwrap();
        let (mut batches, mut total) = (0, 0);
        list_dir(&dir, false, |b| {
            batches += 1;
            total += b.len();
        })
        .unwrap();
        assert_eq!(total, 5000);
        assert!(batches >= 1);
    }

    #[test]
    fn sample_stops_at_the_visible_cap_and_reports_partial() {
        let dir = temp_dir("sample-cap");
        for i in 0..8 {
            std::fs::write(dir.join(format!("file-{i}")), "x").unwrap();
        }
        std::fs::write(dir.join(".hidden-a"), "").unwrap();
        std::fs::write(dir.join(".hidden-b"), "").unwrap();
        let cancel = crate::sizes::Cancel::new();
        let sample = list_sample(&dir, false, 3, None, &cancel).unwrap();
        assert_eq!(sample.entries.len(), 3);
        assert!(sample.hidden_examined <= sample.examined, "hidden entries are counted as examined");
        assert!(matches!(sample.incomplete, Some(SampleIncomplete::LimitReached { .. })));

        let complete = list_sample(&dir, false, 20, Some(100), &cancel).unwrap();
        assert_eq!(complete.entries.len(), 8);
        assert!(complete.hidden_examined >= 2);
        assert_eq!(complete.incomplete, None);
    }

    #[test]
    fn sample_stops_at_the_examined_cap_and_at_cancel() {
        let dir = temp_dir("sample-examined");
        for i in 0..20 {
            std::fs::write(dir.join(format!(".dot-{i}")), "").unwrap();
        }
        let cancel = crate::sizes::Cancel::new();
        let sample = list_sample(&dir, false, 8, Some(4), &cancel).unwrap();
        assert!(sample.entries.is_empty());
        assert_eq!(sample.examined, 4);
        assert!(matches!(sample.incomplete, Some(SampleIncomplete::ExaminedLimit { examined: 4 })));

        cancel.cancel();
        let stopped = list_sample(&dir, true, 8, Some(100), &cancel).unwrap();
        assert!(stopped.entries.is_empty());
        assert!(matches!(stopped.incomplete, Some(SampleIncomplete::Cancelled { .. })));
    }

    #[test]
    fn sample_confirms_an_empty_folder() {
        let dir = temp_dir("sample-empty");
        let sample = list_sample(&dir, true, 12, Some(64), &crate::sizes::Cancel::new()).unwrap();
        assert!(sample.entries.is_empty());
        assert_eq!(sample.examined, 0);
        assert_eq!(sample.incomplete, None);
    }

    #[test]
    fn missing_folder_is_an_error() {
        assert!(list_dir(Path::new("/definitely/not/here"), false, |_| {}).is_err());
    }
}
