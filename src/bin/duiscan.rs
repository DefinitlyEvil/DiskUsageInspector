//! Headless scan harness (console subsystem, so it can print timings).
//!
//! ```text
//! duiscan <path> [--on-disk] [--no-files] [--threads N] [--top N]
//! ```
//!
//! Read-only, like the GUI: it enumerates metadata and prints a summary.

use std::sync::atomic::Ordering;
use std::time::Instant;

use disk_usage_inspector::{format, scan};

fn main() {
    let mut path: Option<String> = None;
    let mut on_disk = false;
    let mut include_files = true;
    let mut threads = scan::default_threads();
    let mut top = 15usize;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--on-disk" => on_disk = true,
            "--no-files" => include_files = false,
            "--threads" => {
                i += 1;
                threads = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(threads);
            }
            "--top" => {
                i += 1;
                top = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(top);
            }
            other => path = Some(other.to_string()),
        }
        i += 1;
    }
    let Some(path) = path else {
        eprintln!("usage: duiscan <path> [--on-disk] [--no-files] [--threads N] [--top N]");
        std::process::exit(2);
    };

    let scan_root = if path.starts_with("\\\\?\\") {
        path.clone()
    } else if path.len() == 2 && path.ends_with(':') {
        format!("\\\\?\\{path}\\")
    } else {
        format!("\\\\?\\{path}")
    };

    let started = Instant::now();
    let (shared, handles) = scan::start(
        &path,
        &scan_root,
        scan::Options {
            on_disk,
            include_files,
        },
        threads,
    );

    // Progress monitor so a stall (or a deadlock) is visible instead of silent.
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = {
        let s = std::sync::Arc::clone(&shared);
        let f = std::sync::Arc::clone(&finished);
        std::thread::spawn(move || {
            let mut last = 0u64;
            let mut stalled = 0u32;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if f.load(Ordering::Relaxed) {
                    return;
                }
                let files = s.progress.files.load(Ordering::Relaxed);
                let dirs = s.progress.dirs.load(Ordering::Relaxed);
                eprintln!(
                    "  [{:>6.1}s] {:>9} files {:>8} dirs",
                    started.elapsed().as_secs_f64(),
                    files,
                    dirs
                );
                if files == last {
                    stalled += 1;
                    if stalled.is_multiple_of(5) {
                        eprintln!("  !! no file progress for {}s", stalled * 2);
                    }
                } else {
                    stalled = 0;
                }
                last = files;
            }
        })
    };
    for h in handles {
        let _ = h.join();
    }
    // Stop the clock as soon as the workers are done; the monitor sleeps in 2 s
    // steps and joining it must not be billed to the scan.
    let elapsed = started.elapsed();
    finished.store(true, Ordering::Relaxed);
    let _ = monitor.join();

    let m = shared.model.lock().unwrap();
    let root = m.node(m.root);
    println!("path        {}", path);
    println!("elapsed     {:.3} s", elapsed.as_secs_f64());
    println!(
        "threads     {}  (on_disk={on_disk}, include_files={include_files})",
        threads
    );
    println!("files       {}", format::count(root.files));
    println!("folders     {}", format::count(root_dirs(&m)));
    println!(
        "logical     {} ({} bytes)",
        format::bytes(root.size),
        root.size
    );
    if on_disk {
        println!(
            "on disk     {} ({} bytes)",
            format::bytes(root.on_disk),
            root.on_disk
        );
    }
    println!("nodes       {}", m.nodes.len());
    println!(
        "unreadable  {}",
        shared.progress.errors.load(Ordering::Relaxed)
    );
    println!(
        "throughput  {:.0} files/s, {:.0} Mtok/s-ish",
        root.files as f64 / elapsed.as_secs_f64().max(1e-6),
        root.size as f64 / elapsed.as_secs_f64().max(1e-6) / 1e6
    );

    let mut kids: Vec<_> = root.children.iter().map(|&c| m.node(c)).collect();
    kids.sort_by_key(|a| std::cmp::Reverse(a.size));
    println!("\ntop {top} entries:");
    for n in kids.iter().take(top) {
        let pct = if root.size > 0 {
            n.size as f64 * 100.0 / root.size as f64
        } else {
            0.0
        };
        println!(
            "  {:>10}  {:>5.1}%  {}{}",
            format::bytes(n.size),
            pct,
            n.name,
            if n.is_dir { "\\" } else { "" }
        );
    }
}

fn root_dirs(m: &disk_usage_inspector::model::Model) -> u64 {
    let mut n = 0u64;
    let mut stack = vec![m.root];
    while let Some(id) = stack.pop() {
        for &c in &m.node(id).children {
            if m.node(c).is_dir {
                n += 1;
                stack.push(c);
            }
        }
    }
    n
}
