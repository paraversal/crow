//! Recently used folders, ranked by frecency (how often and how recently), the
//! way zoxide ranks them. When zoxide is installed its list is merged in, so
//! folders you `cd` to in a terminal show up too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;
/// Like zoxide's `_ZO_MAXAGE`: past this total, every rank is scaled down and the
/// faintest entries are forgotten.
const MAX_TOTAL: f64 = 10_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Visit {
    rank: f64,
    /// Seconds since the Unix epoch.
    last: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    visits: HashMap<PathBuf, Visit>,
}

/// zoxide's weighting: recent visits count more.
fn frecency(visit: &Visit, now: u64) -> f64 {
    let age = now.saturating_sub(visit.last);
    let factor = if age < HOUR {
        4.0
    } else if age < DAY {
        2.0
    } else if age < WEEK {
        0.5
    } else {
        0.25
    };
    visit.rank * factor
}

impl Recent {
    pub fn visit(&mut self, dir: &Path, now: u64) {
        let v = self.visits.entry(dir.to_path_buf()).or_insert(Visit { rank: 0.0, last: now });
        v.rank += 1.0;
        v.last = now;
        let total: f64 = self.visits.values().map(|v| v.rank).sum();
        if total > MAX_TOTAL {
            let scale = 0.9 * MAX_TOTAL / total;
            self.visits.retain(|_, v| {
                v.rank *= scale;
                v.rank >= 1.0
            });
        }
    }

    pub fn forget(&mut self, dir: &Path) {
        self.visits.remove(dir);
    }

    /// The `count` best folders, best first. `zoxide` holds (score, path) from
    /// `zoxide query --list --score`, already weighted, and adds to Crow's own scores.
    /// `keep` filters (folders that still exist and aren't favorites, say).
    pub fn top(&self, count: usize, now: u64, zoxide: &[(f64, PathBuf)], keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        let mut scores: HashMap<&Path, f64> = HashMap::new();
        for (path, visit) in &self.visits {
            *scores.entry(path).or_default() += frecency(visit, now);
        }
        for (score, path) in zoxide {
            *scores.entry(path).or_default() += score;
        }
        let mut ranked: Vec<(&Path, f64)> = scores.into_iter().filter(|(p, _)| keep(p)).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        ranked.into_iter().take(count).map(|(p, _)| p.to_path_buf()).collect()
    }

    pub fn load(path: &Path) -> Recent {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        crate::config::write_atomic(path, &text)
    }
}

/// Parse `zoxide query --list --score`: lines of `  12.5 /some/path`.
pub fn parse_zoxide(output: &str) -> Vec<(f64, PathBuf)> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (score, path) = line.split_once(' ')?;
            Some((score.parse().ok()?, PathBuf::from(path.trim_start())))
        })
        .collect()
}

/// Where zoxide may be. Apps started from the Dock get a minimal `PATH`, so the
/// usual install locations are tried too.
pub fn zoxide_candidates(home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("zoxide")).collect())
        .unwrap_or_default();
    out.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/run/current-system/sw/bin"]
            .iter()
            .map(|d| Path::new(d).join("zoxide")),
    );
    out.extend([".cargo/bin", ".nix-profile/bin", ".local/bin"].iter().map(|d| home.join(d).join("zoxide")));
    out
}

/// Run zoxide if it's installed. Blocking: call it off the main thread.
pub fn query_zoxide(home: &Path) -> Vec<(f64, PathBuf)> {
    let Some(bin) = zoxide_candidates(home).into_iter().find(|p| p.is_file()) else { return Vec::new() };
    match std::process::Command::new(bin).args(["query", "--list", "--score"]).output() {
        Ok(out) if out.status.success() => parse_zoxide(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    #[test]
    fn frequent_and_recent_rank_first() {
        let mut r = Recent::default();
        for _ in 0..5 {
            r.visit(Path::new("/often"), NOW - 2 * WEEK);
        }
        r.visit(Path::new("/just-now"), NOW - 60);
        r.visit(Path::new("/yesterday"), NOW - 30 * HOUR);
        // /often: 5 × 0.25 = 1.25; /just-now: 1 × 4 = 4; /yesterday: 1 × 0.5.
        assert_eq!(
            r.top(10, NOW, &[], |_| true),
            [Path::new("/just-now"), Path::new("/often"), Path::new("/yesterday")]
        );
        assert_eq!(r.top(1, NOW, &[], |_| true).len(), 1);
        assert_eq!(r.top(10, NOW, &[], |p| p != Path::new("/often")).len(), 2, "keep filters");
    }

    #[test]
    fn zoxide_scores_add_to_crows() {
        let mut r = Recent::default();
        r.visit(Path::new("/a"), NOW);
        let z = parse_zoxide("  10.0 /b\n   3.5 /a\nnot a line\n  2 /with space/x\n");
        assert_eq!(z.len(), 3);
        assert_eq!(z[2], (2.0, PathBuf::from("/with space/x")));
        // /a: 4 + 3.5 = 7.5 < /b: 10.
        assert_eq!(r.top(2, NOW, &z, |_| true), [Path::new("/b"), Path::new("/a")]);
    }

    #[test]
    fn ages_out_and_round_trips() {
        let mut r = Recent::default();
        r.visit(Path::new("/rare"), NOW);
        for _ in 0..(MAX_TOTAL as usize + 5) {
            r.visit(Path::new("/busy"), NOW);
        }
        assert!(r.visits.contains_key(Path::new("/busy")));
        assert!(!r.visits.contains_key(Path::new("/rare")), "scaled below 1 and forgotten");

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state/recent.json");
        r.save(&file).unwrap();
        assert_eq!(Recent::load(&file), r);
        assert_eq!(Recent::load(&dir.path().join("missing.json")), Recent::default());
    }
}
