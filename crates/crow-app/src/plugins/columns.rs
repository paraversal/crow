//! One plugin column after Name. The descriptor owns the width, minimum and
//! alignment; the ready cell comes from the coordinator's row cache.

use crow_core::{Align, Column, ColumnCell, ColumnContext, Descriptor, ExtensionId};
use crow_fs::{Entry, SortKey};

pub struct PluginColumn {
    id: String,
    extension: ExtensionId,
    title: String,
    width: f32,
    min_width: f32,
    align: Align,
}

impl PluginColumn {
    pub fn new(descriptor: &Descriptor) -> Self {
        Self {
            id: descriptor.id.to_string(),
            extension: descriptor.id.clone(),
            title: descriptor.title.clone(),
            width: f32::from(descriptor.width),
            min_width: f32::from(descriptor.min_width),
            align: descriptor.align,
        }
    }
}

impl Column for PluginColumn {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn min_width(&self) -> f32 {
        self.min_width
    }

    fn visible_by_default(&self) -> bool {
        false
    }

    fn align(&self) -> Align {
        self.align
    }

    fn sort_key(&self) -> Option<SortKey> {
        None
    }

    fn text(&self, _: &Entry, _: &ColumnContext) -> String {
        String::new()
    }

    fn cell(&self, entry: &Entry, cx: &ColumnContext) -> ColumnCell {
        let value = (|| {
            let plugin_value = cx.plugin_value?;
            let version = cx.plugin_version?(entry);
            plugin_value(&self.extension, &entry.path, version)
        })();
        value.unwrap_or_else(|| ColumnCell::plain(String::new()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crow_core::{ExtensionKind, LocalId, PluginId};

    fn descriptor() -> Descriptor {
        let mut descriptor = Descriptor::new(
            ExtensionKind::Column,
            ExtensionId::new(PluginId::parse("example.test").unwrap(), LocalId::parse("lines").unwrap()),
            "Lines".into(),
            crow_core::DescriptorSource::Function("run".into()),
        );
        descriptor.width = 220;
        descriptor.min_width = 80;
        descriptor.align = Align::Right;
        descriptor
    }

    fn entry() -> Entry {
        Entry {
            name: "a.txt".into(),
            path: PathBuf::from("/tmp/a.txt"),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            dataless: false,
            size: 3,
            modified: None,
            created: None,
            added: None,
        }
    }

    #[test]
    fn plugin_column_uses_descriptor_metadata_and_ready_values() {
        let column = PluginColumn::new(&descriptor());
        assert_eq!(column.id(), "example.test.lines");
        assert_eq!(column.title(), "Lines");
        assert_eq!(column.width(), 220.0);
        assert_eq!(column.min_width(), 80.0);
        assert_eq!(column.align(), Align::Right);
        assert!(!column.visible_by_default());
        assert!(column.sort_key().is_none(), "plugin headers are unsortable");

        let value = |_: &ExtensionId, _: &Path, version: u64| {
            (version == 7).then(|| ColumnCell {
                text: "3".into(),
                spoken: Some("three".into()),
                opacity: 1.0,
                fade: None,
            })
        };
        let version = |_: &Entry| 7;
        let kind = |_: &Entry| "Text".into();
        let time = |_: Option<std::time::SystemTime>| String::new();
        let size = |_: &Entry| None;
        let cx = ColumnContext {
            kind: &kind,
            time: &time,
            size: &size,
            plugin_value: Some(&value),
            plugin_version: Some(&version),
        };
        let cell = column.cell(&entry(), &cx);
        assert_eq!(cell.text, "3");
        assert_eq!(cell.spoken.as_deref(), Some("three"));
    }
}
