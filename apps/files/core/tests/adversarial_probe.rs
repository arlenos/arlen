//! TEMPORARY adversarial probe (reviewer-added, to be deleted). Confirms the
//! search walk terminates and stays bounded on hostile structures.
use arlen_file_browser_core::search::{search, SearchOptions, DEFAULT_MAX_DEPTH};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::fs;
use std::time::{Duration, Instant};

fn cap(p: &std::path::Path) -> Dir {
    Dir::open_ambient_dir(p, ambient_authority()).unwrap()
}

// A symlink chain a->b->a (mutual links) plus a self-link, all leaves.
#[test]
fn mutual_and_self_symlinks_terminate_fast() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    fs::create_dir_all(r.join("d")).unwrap();
    fs::write(r.join("d/target.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(r, r.join("d/up")).unwrap();          // -> root
    std::os::unix::fs::symlink(r.join("d"), r.join("d/self")).unwrap(); // -> own dir
    std::os::unix::fs::symlink("self", r.join("d/rel")).unwrap();    // relative loop
    let dir = cap(r);
    let opts = SearchOptions { query: "target".into(), ..Default::default() };
    let t = Instant::now();
    let out = search(&dir, &opts);
    assert!(t.elapsed() < Duration::from_secs(3), "must not hang");
    assert_eq!(out.hits.len(), 1);
}

// Deep nesting beyond max_depth: confirm the depth cap stops descent and the
// walk is bounded even with a very deep real tree.
#[test]
fn deep_tree_is_capped_by_max_depth() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = tmp.path().to_path_buf();
    // Build 40 levels deep, place the needle at the very bottom.
    for i in 0..40 {
        p = p.join(format!("lvl{i}"));
        fs::create_dir(&p).unwrap();
    }
    fs::write(p.join("deep.txt"), b"x").unwrap();
    let dir = cap(tmp.path());
    let opts = SearchOptions { query: "deep".into(), max_depth: DEFAULT_MAX_DEPTH, ..Default::default() };
    let out = search(&dir, &opts);
    // 40 > 16, so the deep file is beyond the cap and never reached.
    assert!(out.hits.is_empty(), "depth cap stops descent below max_depth");
}

// A symlink whose target is a deep real path INSIDE the root: cap-std follows it
// as an intermediate component, but it is a leaf in the walk, so no descent.
#[test]
fn intra_root_symlink_to_deep_dir_is_a_leaf() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    fs::create_dir_all(r.join("real/a/b/c")).unwrap();
    fs::write(r.join("real/a/b/c/found.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(r.join("real/a/b/c"), r.join("shortcut")).unwrap();
    let dir = cap(r);
    let out = search(&dir, &SearchOptions { query: "found".into(), ..Default::default() });
    // Found exactly once via the REAL path; the symlink "shortcut" is a leaf,
    // never descended, so no duplicate "shortcut/found.txt" hit.
    assert_eq!(out.hits.len(), 1, "symlink-to-dir is a leaf, no duplicate via the link");
    assert_eq!(out.hits[0].rel_path, "real/a/b/c/found.txt");
}

// Total content budget: many files, small per-file, confirm total cap engages.
#[test]
fn total_content_budget_engages_and_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    // 20 files of 100 bytes each = 2000 bytes; budget 500 -> not all read.
    for i in 0..20 {
        fs::write(r.join(format!("f{i}.txt")), vec![b'z'; 100]).unwrap();
    }
    let dir = cap(r);
    let opts = SearchOptions {
        query: "needle".into(),
        match_names: false,
        match_content: true,
        max_total_content_bytes: 500,
        ..Default::default()
    };
    let out = search(&dir, &opts);
    assert!(out.content_budget_exhausted, "total budget exhaustion is flagged");
    assert!(out.hits.is_empty());
}

// A needle LONGER than one chunk: the window only ever retains `overlap` =
// needle.len()-1 bytes plus one chunk, so a needle > CONTENT_CHUNK_BYTES still
// accumulates enough window to be found (window grows to chunk+overlap before drain).
#[test]
fn needle_longer_than_a_chunk_is_found() {
    use arlen_file_browser_core::search::{search, SearchOptions};
    let tmp = tempfile::tempdir().unwrap();
    // 64 KiB chunk; make a 100 KiB needle inside a 300 KiB file.
    let chunk = 64 * 1024usize;
    let needle_len = 100 * 1024usize;
    let needle: Vec<u8> = vec![b'N'; needle_len];
    let mut body = vec![b'a'; chunk + 7];
    body.extend_from_slice(&needle);
    body.extend_from_slice(&[b'b'; 1234]);
    fs::write(tmp.path().join("huge_needle.bin"), &body).unwrap();
    let q = String::from_utf8(needle).unwrap();
    let dir = cap(tmp.path());
    let opts = SearchOptions {
        query: q,
        match_names: false,
        match_content: true,
        max_file_bytes: 10 * 1024 * 1024,
        ..Default::default()
    };
    let out = search(&dir, &opts);
    assert_eq!(out.hits.len(), 1, "a needle larger than the chunk is still found");
}
