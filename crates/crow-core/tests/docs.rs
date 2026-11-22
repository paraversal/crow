//! `docs/configuration.md` embeds each config file as Crow writes it on first launch, so the
//! reference cannot drift from the app. `CROW_UPDATE_DOCS=1 cargo test -p crow-core --test docs`
//! rewrites the blocks.

use std::path::Path;

fn defaults() -> Vec<(&'static str, &'static str)> {
    vec![
        ("settings.toml", crow_core::DEFAULT_SETTINGS),
        ("views.toml", crow_core::DEFAULT_VIEWS),
        ("sidebar.toml", crow_core::DEFAULT_SIDEBAR),
        ("exclude.toml", crow_core::DEFAULT_EXCLUDE),
        ("saved-queries.toml", crow_core::DEFAULT_SAVED_QUERIES),
        ("keys.toml", crow_core::keymap_edit::DEFAULT_KEYS),
    ]
}

#[test]
fn configuration_doc_embeds_the_shipped_defaults() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/configuration.md");
    let mut doc = std::fs::read_to_string(&path).expect("docs/configuration.md");
    for (name, text) in defaults() {
        let open = format!("<!-- defaults:{name} -->\n");
        let close = "<!-- /defaults -->";
        let start = doc.find(&open).unwrap_or_else(|| panic!("marker for {name}")) + open.len();
        let end = start + doc[start..].find(close).unwrap_or_else(|| panic!("closing marker for {name}"));
        let block = format!("```toml\n{}\n```\n", text.trim_end());
        if std::env::var_os("CROW_UPDATE_DOCS").is_some() {
            doc.replace_range(start..end, &block);
        } else {
            assert_eq!(
                &doc[start..end],
                block,
                "docs/configuration.md is out of date for {name}: run CROW_UPDATE_DOCS=1 cargo test -p crow-core --test docs"
            );
        }
    }
    if std::env::var_os("CROW_UPDATE_DOCS").is_some() {
        std::fs::write(&path, doc).unwrap();
    }
}
