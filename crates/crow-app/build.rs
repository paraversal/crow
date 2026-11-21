//! Stamps the build with its commit count and short hash (`CROW_BUILD`, `CROW_COMMIT`).
//! `scripts/bundle.sh` may set both in the environment (a source tarball has no `.git`);
//! otherwise git is asked, and a build outside a checkout says `0` / `unknown`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=CROW_BUILD");
    println!("cargo:rerun-if-env-changed=CROW_COMMIT");
    // A new commit moves HEAD (or the ref it names); that is when the stamp changes.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed=../../.git/{head}");
    }
    let build =
        std::env::var("CROW_BUILD").ok().or_else(|| git(&["rev-list", "--count", "HEAD"])).unwrap_or("0".into());
    let commit = std::env::var("CROW_COMMIT")
        .ok()
        .or_else(|| git(&["rev-parse", "--short=7", "HEAD"]))
        .unwrap_or("unknown".into());
    println!("cargo:rustc-env=CROW_BUILD={build}");
    println!("cargo:rustc-env=CROW_COMMIT={commit}");
}
