//! Crow's public plugin runtime: versioned Rhai packages, a deny-by-default
//! grants store, one budgeted worker per enabled package and the validated
//! extension registry. GPUI-free; the app owns presentation and context.

pub mod dto;
mod excerpt;
pub mod grants;
pub mod host;
pub mod limits;
pub mod manifest;
pub mod package;
pub mod registry;
mod worker;

pub use dto::{EntryRef, ExcerptInput, HostEvent, Inventory, PluginEntry, PluginState, RequestInput};
pub use excerpt::read_excerpt;
pub use grants::{
    DEFAULT_PLUGINS, Grant, Grants, approve, ensure_grants_file, grants_path, load_grants, parse_grants, set_enabled,
    withdraw,
};
pub use host::{PluginHandle, PluginService};
pub use limits::Limits;
pub use manifest::{
    API_VERSION, FsAccess, Manifest, PackageError, PackageErrorKind, Permissions, parse_manifest, validate_relative,
};
pub use package::{
    Discovery, PackageCandidate, PackageSnapshot, SUPPORTED_API, bundle_path, digest_files, discover, load_package,
    load_package_at,
};
