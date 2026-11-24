//! `crow --grant-bundled git --config-dir DIR`: a developer flag for isolated runs
//! (screenshot scripts, component shots, tests). It approves the embedded Git
//! package in `DIR/plugins.toml` through the same `grants::approve` the Settings
//! approval uses, with the digest of the bytes compiled into this binary, so a
//! changed bundle is granted afresh and nothing is hard-coded.
//!
//! It never touches the user's real config: `DIR` must be an existing folder
//! under the system temp folder or under `$CROW_SHOT_HOME`, and must not be a
//! folder the app would use as its own config. The normal approval flow is not
//! involved: a real run still asks.

use std::path::{Path, PathBuf};

/// Approve the embedded bundled package `name` (only `git` exists) in `config_dir`.
pub fn grant_bundled(name: &str, config_dir: &Path) -> Result<(), String> {
    if name != crate::git::PLUGIN_ID {
        return Err(format!("no bundled package \"{name}\" (only \"git\")"));
    }
    let dir = isolated(config_dir)?;
    let package = crate::plugins::git_package()?;
    crow_plugin::grants::approve(&crow_plugin::grants::grants_path(&dir), &package.manifest.id, &package)
}

/// `dir`, canonical, when it is safe to write: a temp or shot-home folder that is not a real config dir.
fn isolated(dir: &Path) -> Result<PathBuf, String> {
    let mut roots: Vec<PathBuf> = vec![std::env::temp_dir(), PathBuf::from("/tmp")];
    if let Some(shot_home) = std::env::var_os("CROW_SHOT_HOME") {
        roots.push(PathBuf::from(shot_home));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut real = vec![crow_core::config_dir(), home.join(".config").join("crow")];
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        real.push(PathBuf::from(xdg).join("crow"));
    }
    isolated_in(dir, &roots, &real)
}

fn isolated_in(dir: &Path, roots: &[PathBuf], real: &[PathBuf]) -> Result<PathBuf, String> {
    // io-ok: developer flag, checks the target before writing
    let canonical = dir.canonicalize().map_err(|e| format!("{}: {e} (the folder must exist)", dir.display()))?;
    let under_root = roots
        .iter()
        // io-ok: developer flag
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical.starts_with(&root) && canonical != root);
    if !under_root {
        return Err(format!(
            "{}: refusing to write outside the system temp folder or $CROW_SHOT_HOME",
            canonical.display()
        ));
    }
    // io-ok: developer flag
    if real.iter().filter_map(|path| path.canonicalize().ok()).any(|path| path == canonical) {
        return Err(format!("{}: refusing to write the real config folder", canonical.display()));
    }
    Ok(canonical)
}

/// The `--grant-bundled NAME --config-dir DIR` entry point; the exit code.
pub fn run(args: &[String]) -> std::process::ExitCode {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1));
    let (Some(name), Some(dir)) = (value("--grant-bundled"), value("--config-dir")) else {
        eprintln!("usage: crow --grant-bundled git --config-dir DIR");
        return std::process::ExitCode::FAILURE;
    };
    match grant_bundled(name, Path::new(dir)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("crow --grant-bundled: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granted(dir: &Path) -> crow_plugin::Grants {
        crow_plugin::grants::load_grants(&crow_plugin::grants::grants_path(dir)).expect("plugins.toml parses")
    }

    #[test]
    fn the_grant_matches_the_embedded_digest() {
        let dir = tempfile::tempdir().unwrap();
        grant_bundled("git", dir.path()).unwrap();
        let package = crate::plugins::git_package().unwrap();
        let grants = granted(dir.path());
        assert!(grants.authorizing(&package.manifest.id, &package).is_some());
    }

    #[test]
    fn a_changed_bundle_is_pending_again() {
        let dir = tempfile::tempdir().unwrap();
        grant_bundled("git", dir.path()).unwrap();
        let package = crate::plugins::git_package().unwrap();
        let mut changed = (*package).clone();
        changed.digest = "0".repeat(64);
        assert!(granted(dir.path()).authorizing(&package.manifest.id, &changed).is_none());
    }

    #[test]
    fn it_refuses_the_real_config_and_unknown_packages() {
        // The user's real folder is outside the temp folder.
        assert!(isolated(&crow_core::config_dir()).is_err());
        assert!(isolated(Path::new("/Users")).is_err());
        assert!(isolated(Path::new("/nonexistent-crow-dir")).is_err());
        assert!(grant_bundled("other", Path::new("/tmp")).is_err());
        let temp = tempfile::tempdir().unwrap();
        assert!(isolated(temp.path()).is_ok());
    }

    #[test]
    fn a_temp_folder_that_is_the_real_config_is_refused() {
        // `CROW_CONFIG_DIR=/tmp/x` makes /tmp/x the real config: still refused.
        let temp = tempfile::tempdir().unwrap();
        let roots = [std::env::temp_dir()];
        assert!(isolated_in(temp.path(), &roots, &[temp.path().to_path_buf()]).unwrap_err().contains("real config"));
        assert!(isolated_in(temp.path(), &roots, &[]).is_ok());
        // The root itself is not a target, only folders below it.
        assert!(isolated_in(&std::env::temp_dir(), &roots, &[]).is_err());
    }
}
