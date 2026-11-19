//! The app's extension registry: crow-core's built-ins plus the ones that need macOS.

use std::sync::Arc;

use crow_core::{Extensions, PreviewContent, Previewer};
use crow_fs::Entry;
use crow_fs::{ReadDecision, ReadGate, ReadHint};
use gpui::{App, Global};

pub struct Ext(pub Arc<Extensions>);

impl Global for Ext {}

pub fn init(cx: &mut App) {
    let mut extensions = Extensions::with_builtins();
    extensions.register_previewer(Arc::new(QuickLookThumbnail));
    cx.set_global(Ext(Arc::new(extensions)));
}

/// Quick Look's thumbnail: images, PDFs, documents and anything else with a Quick Look plugin.
/// Registered after the text previewer, so text files show as text.
struct QuickLookThumbnail;

impl Previewer for QuickLookThumbnail {
    fn id(&self) -> &'static str {
        "quicklook-thumbnail"
    }

    fn preview(&self, entry: &Entry, gate: &dyn ReadGate, image_points: u32) -> Option<PreviewContent> {
        if entry.is_dir && !entry.is_package() {
            return None;
        }
        if gate.check(&entry.path, ReadHint::for_path(&entry.path)) != ReadDecision::Readable {
            return None;
        }
        crow_macos::thumbnail_png(&entry.path, f64::from(image_points)).map(|png| PreviewContent::Image(Arc::new(png)))
    }
}
