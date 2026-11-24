//! Settings search: one query over the same catalog the category pages draw.
//! Exact labels, aliases and backing keys rank ahead of prefixes, category names
//! and fuzzy matches; ties follow category and catalog order so results never
//! shuffle between keystrokes.

use crow_search::FuzzyMatcher;

use super::ControlId;
use super::catalog::{self, Category, Unit};

/// Weak subsequence matches below this score (`vim` against `views.toml`, say)
/// are noise for a settings list; exact, prefix and category results are not
/// filtered.
const MIN_FUZZY_SCORE: u32 = 75;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Hit {
    pub unit: ControlId,
    pub category: Category,
}

/// Matching units in deterministic result order. An empty query returns none;
/// the caller shows the active category instead.
pub(super) fn matches(query: &str) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut fuzzy = FuzzyMatcher::new(&needle);
    let mut scored: Vec<(u8, u32, usize, usize, &'static Unit)> = Vec::new();
    for (unit_index, unit) in catalog::UNITS.iter().enumerate() {
        let label = unit.label.to_lowercase();
        let category = unit.category.label().to_lowercase();
        let aliases = unit.aliases.iter().map(|alias| alias.to_lowercase()).collect::<Vec<_>>();
        let keys = unit.keys.iter().map(|key| key.to_lowercase()).collect::<Vec<_>>();
        let exact = label == needle || aliases.contains(&needle) || keys.contains(&needle);
        let label_prefix = label.starts_with(&needle);
        let category_prefix = category.starts_with(&needle);
        let alias_prefix =
            aliases.iter().any(|alias| alias.starts_with(&needle)) || keys.iter().any(|key| key.starts_with(&needle));
        let score =
            std::iter::once(&label).chain(aliases.iter()).chain(keys.iter()).filter_map(|text| fuzzy.score(text)).max();
        // A category name includes all its units only when the query is its
        // prefix; a fuzzy substring of the name is not a category search.
        let (rank, score) = if exact {
            (0, 0)
        } else if label_prefix {
            (1, 0)
        } else if alias_prefix {
            (2, 0)
        } else if category_prefix {
            (3, 0)
        } else if let Some(score) = score.filter(|score| *score >= MIN_FUZZY_SCORE) {
            (4, score)
        } else {
            continue;
        };
        let category_index = Category::ALL.iter().position(|candidate| *candidate == unit.category).unwrap_or(0);
        scored.push((rank, score, category_index, unit_index, unit));
    }
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)).then_with(|| a.3.cmp(&b.3))
    });
    scored.into_iter().map(|(_, _, _, _, unit)| Hit { unit: unit.id, category: unit.category }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(query: &str) -> Vec<ControlId> {
        matches(query).into_iter().map(|hit| hit.unit).collect()
    }

    #[test]
    fn exact_labels_aliases_and_keys_come_first() {
        assert_eq!(units("icon_pack"), [ControlId::IconPack]);
        assert_eq!(units("vim"), [ControlId::Keymap]);
        assert_eq!(units("return"), [ControlId::Enter]);
        let settings = units("views.toml");
        assert!(settings.contains(&ControlId::DefaultView), "{settings:?}");
        assert!(settings.contains(&ControlId::EditSettings), "{settings:?}");
    }

    #[test]
    fn label_prefixes_beat_fuzzy_matches() {
        let hits = matches("sho");
        assert_eq!(hits.first().map(|hit| hit.unit), Some(ControlId::ShowHidden), "{hits:?}");
        assert_eq!(hits.first().map(|hit| hit.category), Some(Category::Browsing));
    }

    #[test]
    fn category_names_include_their_units() {
        let search = units("search");
        assert!(search.contains(&ControlId::SearchHidden));
        assert!(search.contains(&ControlId::SearchMatch));
        let plugins = units("plugins");
        assert_eq!(plugins.first(), Some(&ControlId::Plugins), "the exact label ranks first");
        assert!(plugins.contains(&ControlId::EditSettings), "the plugins.toml link stays findable");
    }

    #[test]
    fn results_are_stable_and_grouped_in_category_order() {
        let hits = matches("preview");
        let categories: Vec<Category> = hits.iter().map(|hit| hit.category).collect();
        let mut sorted = categories.clone();
        sorted.sort_by_key(|category| Category::ALL.iter().position(|c| c == category));
        assert_eq!(categories, sorted, "categories keep sidebar order");
        assert_eq!(hits, matches("preview"), "typing the same query keeps the order");
        let preview = units("preview");
        assert!(preview.contains(&ControlId::ShowPreview));
        assert!(preview.contains(&ControlId::PreviewScope));
        assert!(
            !preview.contains(&ControlId::DefaultView),
            "a fuzzy category substring does not select the whole category"
        );
        assert!(!preview.contains(&ControlId::ShowHidden));
        assert_eq!(matches("zzzz-nothing").len(), 0);
        assert_eq!(matches("   ").len(), 0);
    }

    #[test]
    fn every_unit_is_findable_by_its_exact_backing_key() {
        for unit in catalog::UNITS {
            for key in unit.keys {
                assert!(units(key).contains(&unit.id), "key {key:?} should find {:?}, got {:?}", unit.id, units(key));
            }
        }
    }
}
