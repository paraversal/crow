//! cargo run --release -p crow-fs --example list_bench -- DIR
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: list_bench DIR");
    let dir = std::path::Path::new(&dir);
    for _ in 0..3 {
        let t = Instant::now();
        let mut entries = Vec::new();
        let mut first_batch = None;
        crow_fs::list_dir(dir, false, |b| {
            first_batch.get_or_insert(t.elapsed());
            entries.extend(b)
        })
        .expect("list");
        let listed = t.elapsed();
        let kind = |e: &crow_fs::Entry| Arc::<str>::from(e.ext());
        crow_fs::sort_entries(&mut entries, &crow_fs::Sort::default(), &kind);
        let sorted = t.elapsed();
        let mut by_kind = entries.clone();
        let tk = Instant::now();
        crow_fs::sort_entries(&mut by_kind, &crow_fs::Sort::default().toggled(crow_fs::SortKey::Kind), &kind);
        println!(
            "{} entries: first batch {:.1} ms, listed {:.1} ms, sorted by name {:.1} ms (total {:.1} ms); kind sort {:.1} ms",
            entries.len(),
            first_batch.unwrap_or_default().as_secs_f64() * 1e3,
            listed.as_secs_f64() * 1e3,
            (sorted - listed).as_secs_f64() * 1e3,
            sorted.as_secs_f64() * 1e3,
            tk.elapsed().as_secs_f64() * 1e3,
        );
    }
}
