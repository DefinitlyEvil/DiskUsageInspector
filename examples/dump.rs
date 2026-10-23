//! Debug helper: `cargo run --example dump -- <path>` prints the scanned tree.

use std::sync::atomic::Ordering;

use disk_usage_inspector::model::Model;
use disk_usage_inspector::scan;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let scan_root = if path.starts_with("\\\\?\\") {
        path.clone()
    } else if path.len() == 2 && path.ends_with(':') {
        format!("\\\\?\\{path}\\")
    } else {
        format!("\\\\?\\{path}")
    };
    let (shared, handles) = scan::start(
        &path,
        &scan_root,
        scan::Options {
            on_disk: false,
            include_files: true,
        },
        4,
    );
    for h in handles {
        let _ = h.join();
    }

    let mut pattern = scan_root.clone();
    if !pattern.ends_with('\\') {
        pattern.push('\\');
    }
    pattern.push('*');
    let wide: Vec<u16> = pattern.encode_utf16().collect();
    match disk_usage_inspector::platform::enumerate_dir(&wide) {
        Ok(v) => println!("direct enumerate: {} entries", v.len()),
        Err(e) => println!("direct enumerate failed: code {e}"),
    }
    let m = shared.model.lock().unwrap();
    walk(&m, m.root, 0);
    println!(
        "done={} errors={}",
        shared.done.load(Ordering::Relaxed),
        shared.progress.errors.load(Ordering::Relaxed)
    );
}

fn walk(m: &Model, id: u32, depth: usize) {
    let n = m.node(id);
    println!(
        "{}{}  [{} bytes, {} files, pending {}]",
        "  ".repeat(depth),
        n.name,
        n.size,
        n.files,
        n.pending
    );
    for &c in &n.children {
        walk(m, c, depth + 1);
    }
}
