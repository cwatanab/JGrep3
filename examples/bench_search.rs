use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".".into());
    let query = std::env::args().nth(2).unwrap_or_else(|| "fn ".into());
    let file_mask = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "*.rs".into());
    let dir_mask = std::env::args()
        .nth(4)
        .unwrap_or_else(|| "**;!.git;!target;!.worktrees".into());

    let t0 = Instant::now();
    let (elapsed_ms, scanned, matches) = jgrep3::search::run_search_headless(
        PathBuf::from(&dir),
        query.clone(),
        file_mask,
        dir_mask,
        true,
        true,
        false,
        false,
    );
    let wall = t0.elapsed().as_secs_f64();

    println!("query : {query}");
    println!("elapsed: {elapsed_ms} ms (wall: {wall:.3}s)");
    println!("scanned: {scanned}");
    println!("matches: {matches}");
}
