//! What `crow --version`, the log's first line and the About panel say about this build.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("CROW_BUILD");
pub const COMMIT: &str = env!("CROW_COMMIT");

/// `Crow 0.1.0 (build 412, 1a2b3c4)`.
pub fn line() -> String {
    format!("Crow {VERSION} (build {BUILD}, {COMMIT})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_version_build_and_commit() {
        let line = line();
        assert!(line.starts_with(&format!("Crow {}", env!("CARGO_PKG_VERSION"))), "{line}");
        assert!(line.contains("build ") && line.ends_with(')'), "{line}");
    }
}
