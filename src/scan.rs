//! Background volume scanner.
//!
//! One fixed worker pool drains a queue of directory tasks. A worker enumerates
//! one directory with `FindFirstFileExW`, registers the entries in the model in a
//! single short lock, then pushes child directories back onto the queue.
//!
//! Why this shape:
//! * no lock per file — only one lock per directory;
//! * no per-directory thread (syscall storm, memory);
//! * cancellation is an `AtomicBool` checked inside the entry loop;
//! * progress lives in atomics, so the UI never receives a per-file event.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::model::{Model, NodeId};
use crate::platform;

#[derive(Clone, Copy)]
pub struct Options {
    /// Collect allocated bytes (one extra syscall per file).
    pub on_disk: bool,
    /// Keep a node per file so the UI can show them. When false, file bytes are
    /// folded into the parent directory only.
    pub include_files: bool,
}

pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
}

impl Progress {
    fn new() -> Self {
        Self {
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

struct Task {
    node: NodeId,
    dir: Vec<u16>,
}

struct Queue {
    tasks: VecDeque<Task>,
    in_flight: usize,
    finished: bool,
}

pub struct Shared {
    pub model: Mutex<Model>,
    pub progress: Progress,
    pub cancel: AtomicBool,
    pub done: AtomicBool,
    /// Number of in-flight lazily started file-list loads (UI keeps repainting
    /// while this is non-zero).
    pub lazy_loads: AtomicUsize,
    opts: Options,
    queue: Mutex<Queue>,
    cv: Condvar,
}

fn lock_model(shared: &Shared) -> std::sync::MutexGuard<'_, Model> {
    shared.model.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock_queue(shared: &Shared) -> std::sync::MutexGuard<'_, Queue> {
    shared.queue.lock().unwrap_or_else(|e| e.into_inner())
}

/// Start a scan. `display_name` becomes the root node label; `scan_root` is the
/// `\\?\C:\` style prefix path.
pub fn start(
    display_name: &str,
    scan_root: &str,
    opts: Options,
    threads: usize,
) -> (Arc<Shared>, Vec<JoinHandle<()>>) {
    let shared = Arc::new(Shared {
        model: Mutex::new(Model::new(display_name)),
        progress: Progress::new(),
        cancel: AtomicBool::new(false),
        done: AtomicBool::new(false),
        lazy_loads: AtomicUsize::new(0),
        opts,
        queue: Mutex::new(Queue {
            tasks: VecDeque::new(),
            in_flight: 0,
            finished: false,
        }),
        cv: Condvar::new(),
    });

    {
        let mut q = lock_queue(&shared);
        let mut root_dir = scan_root.to_string();
        if !root_dir.ends_with('\\') {
            root_dir.push('\\');
        }
        q.tasks.push_back(Task {
            node: 0,
            dir: root_dir.encode_utf16().collect(),
        });
        q.in_flight = 1;
    }

    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads.max(1) {
        let s = Arc::clone(&shared);
        handles.push(std::thread::spawn(move || worker(s)));
    }
    (shared, handles)
}

fn worker(shared: Arc<Shared>) {
    loop {
        let task = {
            let mut q = lock_queue(&shared);
            loop {
                if shared.cancel.load(Ordering::Relaxed) {
                    q.finished = true;
                    shared.done.store(true, Ordering::Release);
                    shared.cv.notify_all();
                    return;
                }
                if let Some(t) = q.tasks.pop_front() {
                    break t;
                }
                if q.in_flight == 0 {
                    q.finished = true;
                    shared.done.store(true, Ordering::Release);
                    shared.cv.notify_all();
                    return;
                }
                q = shared.cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };

        process(&shared, task);

        let mut q = lock_queue(&shared);
        q.in_flight = q.in_flight.saturating_sub(1);
        if q.in_flight == 0 && q.tasks.is_empty() {
            q.finished = true;
            shared.done.store(true, Ordering::Release);
        }
        shared.cv.notify_all();
    }
}

fn process(shared: &Arc<Shared>, task: Task) {
    if shared.cancel.load(Ordering::Relaxed) {
        return;
    }
    let mut pattern = Vec::with_capacity(task.dir.len() + 1);
    pattern.extend_from_slice(&task.dir);
    pattern.push(u16::from(b'*'));

    let entries = match platform::enumerate_dir(&pattern) {
        Ok(v) => v,
        Err(code) => {
            let mut m = lock_model(shared);
            m.fail_dir(task.node, code);
            shared.progress.errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    let mut child_paths: Vec<(NodeId, Vec<u16>)> = Vec::new();
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut dirs = 0u64;
    let mut file_path = task.dir.clone();

    {
        let mut m = lock_model(shared);
        for entry in entries {
            if shared.cancel.load(Ordering::Relaxed) {
                break;
            }
            if entry.is_dir {
                dirs += 1;
                let name: Arc<str> = Arc::from(&*entry.name);
                let id = m.add_dir(task.node, name, entry.reparse);
                if entry.reparse {
                    // Never recurse into junctions/symlinks.
                    m.finish_dir(id, 0);
                } else {
                    let mut p = Vec::with_capacity(task.dir.len() + entry.name.len() + 1);
                    p.extend_from_slice(&task.dir);
                    p.extend(entry.name.encode_utf16());
                    p.push(u16::from(b'\\'));
                    child_paths.push((id, p));
                }
            } else {
                let on_disk = if shared.opts.on_disk {
                    file_path.truncate(task.dir.len());
                    file_path.extend(entry.name.encode_utf16());
                    platform::compressed_size(&file_path).unwrap_or(0)
                } else {
                    0
                };
                bytes += entry.size;
                files += 1;
                if shared.opts.include_files {
                    m.add_file(task.node, Arc::from(&*entry.name), entry.size, on_disk);
                } else {
                    m.add_file_bytes(task.node, entry.size, on_disk);
                }
            }
        }
        m.finish_dir(task.node, 0);
    }

    shared.progress.files.fetch_add(files, Ordering::Relaxed);
    shared.progress.bytes.fetch_add(bytes, Ordering::Relaxed);
    shared.progress.dirs.fetch_add(dirs, Ordering::Relaxed);

    if child_paths.is_empty() || shared.cancel.load(Ordering::Relaxed) {
        return;
    }
    let mut q = lock_queue(shared);
    q.in_flight += child_paths.len();
    for (node, dir) in child_paths {
        q.tasks.push_back(Task { node, dir });
    }
    drop(q);
    shared.cv.notify_all();
}

/// Sensible pool size: enough to keep an NVMe queue busy, few enough to leave
/// the UI thread idle. 32 threads on a 16-core box measurably hurt latency.
pub fn default_threads() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    cores.clamp(4, 12)
}
