//! Fixture-based scan tests. These create files under %TEMP% only — never in the
//! repository and never on the volumes the app inspects in normal use.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use disk_usage_inspector::scan::{self, Options};

fn fixture(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dui-fixture-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a/b/c")).unwrap();
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    std::fs::write(dir.join("root.bin"), vec![1u8; 10]).unwrap();
    std::fs::write(dir.join("a/one.bin"), vec![1u8; 1000]).unwrap();
    std::fs::write(dir.join("a/b/two.bin"), vec![1u8; 2345]).unwrap();
    std::fs::write(dir.join("a/b/c/three.bin"), vec![1u8; 100_000]).unwrap();
    dir
}

fn run(dir: &Path) -> (u64, u64, bool) {
    let scan_root = format!("\\\\?\\{}", dir.to_string_lossy());
    let (shared, handles) = scan::start(
        "fixture",
        &scan_root,
        Options {
            on_disk: false,
            include_files: true,
        },
        4,
    );
    for h in handles {
        h.join().unwrap();
    }
    let m = shared.model.lock().unwrap();
    (m.node(m.root).size, m.node(m.root).files, m.root_ready())
}

#[test]
fn totals_match_fixture() {
    let dir = fixture("totals");
    let (size, files, ready) = run(&dir);
    assert!(ready, "root must be finalized");
    assert_eq!(size, 10 + 1000 + 2345 + 100_000);
    assert_eq!(files, 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancellation_is_honoured() {
    let dir = fixture("cancel");
    let scan_root = format!("\\\\?\\{}", dir.to_string_lossy());
    let (shared, handles) = scan::start(
        "fixture",
        &scan_root,
        Options {
            on_disk: false,
            include_files: true,
        },
        4,
    );
    // Cancel immediately: whatever happens, the scan must terminate and report done.
    shared.cancel.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().unwrap();
    }
    assert!(shared.done.load(Ordering::Relaxed));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multiple_drives_are_enumerable() {
    let drives = disk_usage_inspector::platform::list_drives();
    assert!(!drives.is_empty(), "expected at least one lettered drive");
    for d in &drives {
        assert!(d.letter.ends_with(':'));
    }
}
