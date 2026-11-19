//! Typed drop destinations resolved from the geometry each surface drew.
//!
//! The window owns one router: the deepest eligible row wins over its containing
//! folder, and one accepted drop becomes one request.

use std::path::PathBuf;

use crow_core::Slot;

/// Where a drop would land.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropLocation {
    /// A real folder that exists in a listing, sidebar, path segment or result.
    Folder(PathBuf),
    /// The favorite insertion slot between two sidebar rows.
    SidebarInsertion(Slot),
}

/// One resolved destination, with the layout generation it was resolved in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropHit {
    pub location: DropLocation,
    /// Changes when the surface reflows; a timer captured in an older layout
    /// must not spring.
    pub generation: u64,
    /// Whether holding still may open this folder.
    pub spring: bool,
}

impl DropHit {
    pub fn folder(path: PathBuf, generation: u64, spring: bool) -> Self {
        Self { location: DropLocation::Folder(path), generation, spring }
    }

    pub fn insertion(slot: Slot, generation: u64) -> Self {
        Self { location: DropLocation::SidebarInsertion(slot), generation, spring: false }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        match &self.location {
            DropLocation::Folder(path) => Some(path),
            DropLocation::SidebarInsertion(_) => None,
        }
    }
}
