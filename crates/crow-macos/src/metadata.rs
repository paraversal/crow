//! File metadata from the system: kind names, Finder tags, Quick Look thumbnails.
//! Unlike the rest of this crate these are safe off the main thread (and the
//! thumbnail call must run off it, because it waits for Quick Look).

use std::collections::HashMap;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSFileAttributeKey, NSFileGroupOwnerAccountName, NSFileManager,
    NSFileOwnerAccountName, NSSize, NSString, NSURLTagNamesKey,
};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};
use objc2_uniform_type_identifiers::{UTType, UTTypeFolder};

use crate::ns_url;

/// Kind names by (lowercased extension, is folder).
type KindCache = Mutex<HashMap<(String, bool), Arc<str>>>;

static KINDS: LazyLock<KindCache> = LazyLock::new(Default::default);

/// UTType identifiers by (lowercased extension, is folder), including misses.
type TypeCache = Mutex<HashMap<(String, bool), Option<Arc<[Arc<str>]>>>>;

static TYPES: LazyLock<TypeCache> = LazyLock::new(Default::default);

/// Finder's "Kind" for a file extension (`"Markdown Document"`, `"Folder"`, …),
/// localized by the system and cached per extension.
pub fn kind_name(ext: &str, is_dir: bool) -> Arc<str> {
    let key = (ext.to_lowercase(), is_dir);
    if let Some(hit) = KINDS.lock().ok().and_then(|k| k.get(&key).cloned()) {
        return hit;
    }
    let name: Arc<str> = autoreleasepool(|_| {
        let described = if is_dir && key.0.is_empty() {
            None
        } else {
            UTType::typeWithFilenameExtension(&NSString::from_str(&key.0))
                .filter(|t| !t.isDynamic())
                .and_then(|t| t.localizedDescription())
                .map(|d| d.to_string())
        };
        Arc::from(described.unwrap_or_else(|| fallback_kind(&key.0, is_dir)))
    });
    if let Ok(mut kinds) = KINDS.lock() {
        kinds.insert(key, name.clone());
    }
    name
}

fn fallback_kind(ext: &str, is_dir: bool) -> String {
    match (is_dir, ext) {
        (true, _) => "Folder".into(),
        (false, "") => "Document".into(),
        (false, ext) => format!("{} file", ext.to_uppercase()),
    }
}

/// The system UTType for an extension and every type it conforms to, exact
/// type first (`public.png`, then `public.image`, `public.data`, …). `None`
/// when the system only knows a dynamic type. Cached per extension and safe
/// off the main thread, like `kind_name`.
pub fn type_identifiers(ext: &str, is_folder: bool) -> Option<Arc<[Arc<str>]>> {
    let key = (ext.to_ascii_lowercase(), is_folder);
    if let Some(hit) = TYPES.lock().ok().and_then(|types| types.get(&key).cloned()) {
        return hit;
    }
    let identifiers: Option<Arc<[Arc<str>]>> = autoreleasepool(|_| {
        if is_folder {
            // SAFETY: the UTType constants are immutable for the process lifetime.
            Some(type_chain(unsafe { UTTypeFolder }))
        } else {
            UTType::typeWithFilenameExtension(&NSString::from_str(&key.0))
                .filter(|kind| !kind.isDynamic())
                .map(|kind| type_chain(&kind))
        }
    });
    if let Ok(mut types) = TYPES.lock() {
        types.insert(key, identifiers.clone());
    }
    identifiers
}

fn type_chain(content_type: &UTType) -> Arc<[Arc<str>]> {
    let mut identifiers = vec![Arc::from(content_type.identifier().to_string())];
    for supertype in content_type.supertypes().iter() {
        identifiers.push(Arc::from(supertype.identifier().to_string()));
    }
    identifiers[1..].sort_by(|a, b| type_rank(a).cmp(&type_rank(b)).then_with(|| a.cmp(b)));
    identifiers.dedup();
    Arc::from(identifiers)
}

/// Generic roots sort last so `public.image` wins over `public.data` when a
/// pack maps both.
fn type_rank(identifier: &str) -> u8 {
    match identifier {
        "public.data" | "public.item" | "public.content" => 2,
        _ => 0,
    }
}

/// The item's Finder tags, in Finder's order. Empty when it has none or cannot be read.
///
/// Info's compatibility wrapper: a filter uses [`tags_checked`], because no
/// tags and unreadable metadata are different answers.
pub fn tags(path: &Path) -> Vec<String> {
    tags_checked(path).unwrap_or_default()
}

/// Like [`tags`], but a missing or unreadable path is an error and no tags is
/// `Ok(vec![])`. Works for unindexed paths and hidden files; Finder's tag color
/// is not exposed by this public resource key.
pub fn tags_checked(path: &Path) -> Result<Vec<String>, String> {
    autoreleasepool(|_| {
        let url = ns_url(path);
        let mut value: Option<Retained<AnyObject>> = None;
        // SAFETY: NSURLTagNamesKey yields an NSArray<NSString> (or nil); checked by downcast below.
        unsafe { url.getResourceValue_forKey_error(&mut value, NSURLTagNamesKey) }
            .map_err(|error| format!("{}: {}", path.display(), error.localizedDescription()))?;
        Ok(value
            .and_then(|v| v.downcast::<NSArray>().ok())
            .map(|array| {
                array.iter().filter_map(|item| item.downcast::<NSString>().ok()).map(|s| s.to_string()).collect()
            })
            .unwrap_or_default())
    })
}

/// The mode bits and owner/group names Finder's Get Info shows. Safe off the main
/// thread (Foundation only); `mode` is the permission bits, `file_type` the `ls` letter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub mode: u32,
    pub file_type: char,
    pub owner: Option<String>,
    pub group: Option<String>,
}

impl Permissions {
    /// `drwxr-xr-x`, `-rw-r--r--`, `lrwxr-xr-x`.
    pub fn mode_string(&self) -> String {
        format_mode(self.mode, self.file_type)
    }
}

/// Read `path`'s permissions, following a symlink when its target exists (as Finder
/// does); a dangling link keeps the link's own mode. `None` when neither stat works.
pub fn permissions(path: &Path) -> Option<Permissions> {
    let meta = std::fs::metadata(path).or_else(|_| std::fs::symlink_metadata(path)).ok()?;
    let file_type = file_type_char(meta.file_type());
    let mode = meta.mode() & 0o7777;
    // NSFileManager follows the path; canonicalize first so a symlink reports its target.
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (owner, group) = autoreleasepool(|_| owner_group(&resolved));
    Some(Permissions { mode, file_type, owner, group })
}

fn file_type_char(file_type: std::fs::FileType) -> char {
    if file_type.is_symlink() {
        'l'
    } else if file_type.is_dir() {
        'd'
    } else if file_type.is_block_device() {
        'b'
    } else if file_type.is_char_device() {
        'c'
    } else if file_type.is_fifo() {
        'p'
    } else if file_type.is_socket() {
        's'
    } else {
        '-'
    }
}

fn owner_group(path: &Path) -> (Option<String>, Option<String>) {
    let Some(path) = path.to_str() else { return (None, None) };
    let manager = NSFileManager::defaultManager();
    let Ok(attrs) = manager.attributesOfItemAtPath_error(&NSString::from_str(path)) else {
        return (None, None);
    };
    let read = |key: &NSFileAttributeKey| {
        attrs.objectForKey(key).and_then(|value| value.downcast::<NSString>().ok()).map(|s| s.to_string())
    };
    // SAFETY: Foundation's attribute-key constants are immutable strings for the
    // lifetime of the process.
    let (owner_key, group_key) = unsafe { (NSFileOwnerAccountName, NSFileGroupOwnerAccountName) };
    (read(owner_key), read(group_key))
}

/// The ten-character `ls -l` mode string, including setuid/setgid/sticky.
pub fn format_mode(mode: u32, file_type: char) -> String {
    let mut out = String::with_capacity(10);
    out.push(file_type);
    let bits = [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001];
    let chars = ['r', 'w', 'x', 'r', 'w', 'x', 'r', 'w', 'x'];
    for (i, (bit, ch)) in bits.into_iter().zip(chars).enumerate() {
        let set = mode & bit != 0;
        let special = match i {
            2 => mode & 0o4000 != 0,
            5 => mode & 0o2000 != 0,
            8 => mode & 0o1000 != 0,
            _ => false,
        };
        out.push(if special {
            match (set, i) {
                (true, 2 | 5) => 's',
                (true, _) => 't',
                (false, 2 | 5) => 'S',
                (false, _) => 'T',
            }
        } else if set {
            ch
        } else {
            '-'
        });
    }
    out
}

/// A Quick Look thumbnail for `path` as PNG bytes, at most `points` square (rendered
/// at 2x). Waits up to two seconds; `None` if Quick Look has nothing or takes longer.
/// Returns `None` on the main thread rather than blocking it.
pub fn thumbnail_png(path: &Path, points: f64) -> Option<Vec<u8>> {
    if objc2::MainThreadMarker::new().is_some() {
        return None;
    }
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    let request = autoreleasepool(|_| {
        let url = ns_url(path);
        // SAFETY: valid URL and size; the request is retained for the call.
        let request = unsafe {
            QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
                QLThumbnailGenerationRequest::alloc(),
                &url,
                NSSize::new(points, points),
                2.0,
                QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
            )
        };
        let handler = RcBlock::new(move |representation: *mut QLThumbnailRepresentation, _error: *mut NSError| {
            // SAFETY: Quick Look passes a valid representation or null.
            let png = unsafe { representation.as_ref() }.and_then(|rep| {
                // SAFETY: a representation always carries a CGImage on macOS.
                let image = unsafe { rep.CGImage() };
                let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
                // SAFETY: an empty property dictionary is valid for PNG.
                unsafe { bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
                    .map(|data| data.to_vec())
            });
            tx.send(png).ok();
        });
        // SAFETY: the shared generator is thread-safe; the block is retained by Quick Look.
        unsafe {
            QLThumbnailGenerator::sharedGenerator()
                .generateBestRepresentationForRequest_completionHandler(&request, &handler)
        };
        request
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(png) => png,
        Err(_) => {
            // SAFETY: the request is retained and the shared generator is thread-safe.
            autoreleasepool(|_| unsafe { QLThumbnailGenerator::sharedGenerator().cancelRequest(&request) });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_system() {
        assert_eq!(&*kind_name("", true), "Folder");
        assert_ne!(&*kind_name("md", false), "MD file", "the system knows Markdown");
        assert_eq!(&*kind_name("zzqq", false), "ZZQQ file", "unknown extensions get a readable fallback");
        assert!(Arc::ptr_eq(&kind_name("md", false), &kind_name("MD", false)), "cached case-insensitively");
    }

    #[test]
    fn type_identifiers_come_from_the_system_and_include_supertypes() {
        let folder = type_identifiers("", true).expect("folders have a type");
        assert_eq!(&*folder[0], "public.folder");
        let markdown = type_identifiers("md", false).expect("Markdown has a type");
        assert!(markdown.contains(&Arc::from("public.text")), "{markdown:?}");
        assert!(type_identifiers("zzqq", false).is_none(), "a dynamic type is not a stable pack key");
        assert!(Arc::ptr_eq(
            &type_identifiers("MD", false).expect("cached"),
            &type_identifiers("md", false).expect("cached")
        ));
    }

    #[test]
    fn untagged_and_unreadable_tags_are_distinct() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        assert_eq!(tags_checked(file.path()), Ok(Vec::new()));
        assert!(tags(file.path()).is_empty());
        assert!(tags_checked(Path::new("/no/such/crow-tag-fixture")).is_err());

        // Finder stores tag names in this metadata attribute as a binary plist;
        // xattr writes the fixture without touching Finder or Spotlight.
        let hex = "62706c6973743030a10154576f726b080a000000000000010100000000000000020000000000000000000000000000000f";
        let status = std::process::Command::new("xattr")
            .args(["-wx", "com.apple.metadata:_kMDItemUserTags", hex])
            .arg(file.path())
            .status()
            .expect("run xattr");
        assert!(status.success());
        assert_eq!(tags_checked(file.path()), Ok(vec!["Work".to_owned()]));
    }

    #[test]
    fn mode_strings_match_ls() {
        assert_eq!(format_mode(0o755, 'd'), "drwxr-xr-x");
        assert_eq!(format_mode(0o644, '-'), "-rw-r--r--");
        assert_eq!(format_mode(0o000, '-'), "----------");
        assert_eq!(format_mode(0o777, 'l'), "lrwxrwxrwx");
        assert_eq!(format_mode(0o4755, '-'), "-rwsr-xr-x", "setuid with execute");
        assert_eq!(format_mode(0o4644, '-'), "-rwSr--r--", "setuid without execute");
        assert_eq!(format_mode(0o2755, 'd'), "drwxr-sr-x", "setgid with execute");
        assert_eq!(format_mode(0o1777, 'd'), "drwxrwxrwt", "sticky with execute");
        assert_eq!(format_mode(0o1666, 'd'), "drw-rw-rwT", "sticky without execute");
    }

    #[test]
    fn permissions_read_the_file_and_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o640)).unwrap();
        let permissions = permissions(file.path()).expect("permissions");
        assert_eq!(permissions.mode_string(), "-rw-r-----");
        assert!(permissions.owner.is_some(), "the owner name comes from NSFileManager");
        assert!(permissions.group.is_some(), "the group name comes from NSFileManager");
    }
}
