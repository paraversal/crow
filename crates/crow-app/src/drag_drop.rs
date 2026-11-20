//! Typed drop destinations resolved from the geometry each surface drew.
//!
//! The window owns one router: the deepest eligible row wins over its containing
//! folder, and one accepted drop becomes one request.

use std::path::{Path, PathBuf};

use crow_core::Slot;

/// Where a drop would land.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropLocation {
    /// A real folder that exists in a listing, sidebar, path segment or result.
    Folder(PathBuf),
    /// The favorite insertion slot between two sidebar rows.
    SidebarInsertion(Slot),
    /// The toolbar's Back button: it springs back but never takes a drop.
    Back,
}

/// Which surface resolved a destination; it draws the hover outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DropSurface {
    Back,
    Sidebar,
    PathBar,
    Search,
    #[default]
    Pane,
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
    pub surface: DropSurface,
}

impl DropHit {
    pub fn folder(path: PathBuf, generation: u64, spring: bool) -> Self {
        Self { location: DropLocation::Folder(path), generation, spring, surface: DropSurface::Pane }
    }

    pub fn insertion(slot: Slot, generation: u64) -> Self {
        Self {
            location: DropLocation::SidebarInsertion(slot),
            generation,
            spring: false,
            surface: DropSurface::Sidebar,
        }
    }

    pub fn back(spring: bool) -> Self {
        Self { location: DropLocation::Back, generation: 0, spring, surface: DropSurface::Back }
    }

    pub fn on(mut self, surface: DropSurface) -> Self {
        self.surface = surface;
        self
    }

    /// The spring target holding still here would arm, if any.
    pub fn spring_target(&self) -> Option<crow_core::drag::SpringTarget> {
        if !self.spring {
            return None;
        }
        match &self.location {
            DropLocation::Folder(path) => Some(crow_core::drag::SpringTarget::new(path.clone(), self.generation)),
            DropLocation::Back => Some(crow_core::drag::SpringTarget::Back),
            DropLocation::SidebarInsertion(_) => None,
        }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        match &self.location {
            DropLocation::Folder(path) => Some(path),
            DropLocation::SidebarInsertion(_) | DropLocation::Back => None,
        }
    }
}

/// What an armed spring target looks like right now. The window steps
/// `opacity` through `motion::spring_cue` while one timer is armed; every
/// surface drawing that target's drop outline applies it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpringCue {
    pub target: Option<DropLocation>,
    pub opacity: f32,
}

impl gpui::Global for SpringCue {}

/// The drop outline's opacity for the folder `path`: dimmed only while that
/// folder's spring is armed and the cue is in an off step.
pub fn cue_opacity(cx: &gpui::App, path: &Path) -> f32 {
    match cx.try_global::<SpringCue>() {
        Some(SpringCue { target: Some(DropLocation::Folder(target)), opacity }) if target == path => *opacity,
        _ => 1.0,
    }
}

/// The same for the Back button.
pub fn back_cue_opacity(cx: &gpui::App) -> f32 {
    match cx.try_global::<SpringCue>() {
        Some(SpringCue { target: Some(DropLocation::Back), opacity }) => *opacity,
        _ => 1.0,
    }
}
