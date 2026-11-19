//! Where a tab is: a real folder or a virtual destination. History and tabs
//! store this type, never a path pretending to be a saved search.

use std::path::{Path, PathBuf};

use crate::saved_queries::{SavedQueryId, TagId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Location {
    Folder(PathBuf),
    SavedQuery(SavedQueryId),
    Tag(TagId),
}

impl Location {
    pub fn folder(path: impl Into<PathBuf>) -> Self {
        Self::Folder(path.into())
    }

    pub fn saved_query(id: SavedQueryId) -> Self {
        Self::SavedQuery(id)
    }

    /// The real folder this location shows, if it is one.
    pub fn as_folder(&self) -> Option<&Path> {
        match self {
            Location::Folder(path) => Some(path),
            Location::SavedQuery(_) | Location::Tag(_) => None,
        }
    }

    pub fn saved_query_id(&self) -> Option<&SavedQueryId> {
        match self {
            Location::SavedQuery(id) => Some(id),
            Location::Folder(_) | Location::Tag(_) => None,
        }
    }

    pub fn tag_id(&self) -> Option<&TagId> {
        match self {
            Location::Tag(id) => Some(id),
            Location::Folder(_) | Location::SavedQuery(_) => None,
        }
    }

    pub fn is_folder(&self) -> bool {
        matches!(self, Location::Folder(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_folders_expose_a_path() {
        let folder = Location::Folder(PathBuf::from("/tmp"));
        assert_eq!(folder.as_folder(), Some(Path::new("/tmp")));
        assert!(folder.is_folder());

        let id = SavedQueryId::new("recent-pdfs").unwrap();
        let saved = Location::SavedQuery(id.clone());
        assert_eq!(saved.as_folder(), None);
        assert_eq!(saved.saved_query_id(), Some(&id));
        assert!(!saved.is_folder());

        let tag = Location::Tag(TagId::new("work"));
        assert_eq!(tag.as_folder(), None);
        assert_eq!(tag.tag_id(), Some(&TagId::new("work")));
    }
}
