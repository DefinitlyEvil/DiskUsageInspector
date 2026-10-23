//! eframe/egui UI: drive list, scan progress, virtualized tree.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use egui::{Align, Color32, Layout, RichText, Sense};

use crate::format;
use crate::model::{Model, NodeId};
use crate::platform;
use crate::scan::{self, Shared};

const ROW_H: f32 = 22.0;
const INDENT: f32 = 14.0;
const CHEV_W: f32 = 18.0;
const SIZE_W: f32 = 92.0;
const COUNT_W: f32 = 64.0;
const PCT_W: f32 = 62.0;
const GAP: f32 = 6.0;

struct Session {
    shared: Arc<Shared>,
    handles: Option<Vec<std::thread::JoinHandle<()>>>,
    started: Instant,
    /// Frozen when the scan completes so the status bar stops counting up.
    elapsed: Option<Duration>,
    on_disk: bool,
}

struct Row {
    id: NodeId,
    depth: u16,
    name: Arc<str>,
    size: u64,
    on_disk: u64,
    files: u64,
    pct: f32,
    is_dir: bool,
    expanded: bool,
    has_children: bool,
    reparse: bool,
    errors: u32,
}

pub struct App {
    drives: Vec<platform::DriveInfo>,
    selected_drive: Option<usize>,
    session: Option<Session>,
    expanded: HashSet<NodeId>,
    show_files: bool,
    on_disk: bool,
    rows: Vec<Row>,
    /// (model version, ui epoch) that `rows` was built from.
    rows_key: (u64, u64),
    /// Bumped whenever expansion or a display filter changes.
    ui_epoch: u64,
    last_rebuild: Instant,
    selected_node: Option<NodeId>,
    note: Option<String>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            drives: platform::list_drives(),
            selected_drive: None,
            session: None,
            expanded: HashSet::new(),
            show_files: false,
            on_disk: false,
            rows: Vec::new(),
            rows_key: (0, u64::MAX),
            ui_epoch: 0,
            last_rebuild: Instant::now(),
            selected_node: None,
            note: None,
        }
    }

    fn scanning(&self) -> bool {
        match &self.session {
            Some(s) => !s.shared.done.load(Ordering::Relaxed),
            None => false,
        }
    }

    fn poll(&mut self) {
        let Some(s) = &mut self.session else { return };
        let done = s.shared.done.load(Ordering::Relaxed);
        if done && s.elapsed.is_none() {
            s.elapsed = Some(s.started.elapsed());
        }
        if done && let Some(hs) = s.handles.take() {
            for h in hs {
                let _ = h.join();
            }
        }
    }

    fn start_scan(&mut self, idx: usize) {
        let Some(d) = self.drives.get(idx) else {
            return;
        };
        if d.total == 0 {
            self.note = Some(format!("{} is not ready or has no media", d.letter));
            return;
        }
        if let Some(old) = &self.session {
            old.shared.cancel.store(true, Ordering::Relaxed);
        }
        let opts = scan::Options {
            on_disk: self.on_disk,
            // File nodes are added lazily, per expanded directory, by
            // `ensure_files_loaded`. Keeping them out of the initial scan is worth
            // ~2.3M nodes and ~300 MB on a typical system drive.
            include_files: false,
        };
        let (shared, handles) = scan::start(&d.letter, &d.scan_root, opts, scan::default_threads());
        self.session = Some(Session {
            shared,
            handles: Some(handles),
            started: Instant::now(),
            elapsed: None,
            on_disk: self.on_disk,
        });
        self.selected_drive = Some(idx);
        self.expanded.clear();
        self.expanded.insert(0);
        self.rows.clear();
        self.rows_key = (0, u64::MAX);
        self.ui_epoch = self.ui_epoch.wrapping_add(1);
        self.selected_node = None;
        self.note = None;
        self.last_rebuild = Instant::now() - Duration::from_secs(1);
    }

    fn rebuild_rows(&mut self) {
        let Some(s) = &self.session else {
            self.rows.clear();
            return;
        };
        let shared = Arc::clone(&s.shared);
        let (rows, version) = {
            let m = shared.model.lock().unwrap_or_else(|e| e.into_inner());
            build_rows(&m, &self.expanded, self.show_files)
        };
        self.rows = rows;
        self.rows_key = (version, self.ui_epoch);
        self.last_rebuild = Instant::now();
    }

    fn top_ui(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        let mut rescan = false;
        let mut scan_now: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.heading("Disk Usage Inspector");
            ui.separator();
            if ui.button("Refresh drives").clicked() {
                refresh = true;
            }
            if let Some(i) = self.selected_drive
                && ui.button("Rescan").clicked()
            {
                rescan = true;
                scan_now = Some(i);
            }
            ui.separator();
            let scanning = self.scanning();
            ui.add_enabled_ui(!scanning, |ui| {
                if ui
                    .checkbox(&mut self.on_disk, "Size on disk")
                    .on_hover_text("Allocated bytes (compression/sparse aware). Requires a rescan.")
                    .changed()
                {
                    self.note = Some("On-disk metric applies on the next rescan".into());
                }
            });
            if ui
                .checkbox(&mut self.show_files, "Show files")
                .on_hover_text("Display file rows as well as folders")
                .changed()
            {
                self.ui_epoch = self.ui_epoch.wrapping_add(1);
            }
            if let Some(note) = &self.note {
                ui.separator();
                ui.label(RichText::new(note).weak());
            }
        });
        if refresh {
            self.drives = platform::list_drives();
            self.selected_drive = None;
        }
        let _ = rescan;
        if let Some(i) = scan_now {
            self.start_scan(i);
        }
    }

    fn drives_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("Drives");
        ui.add_space(2.0);
        let mut pick: Option<usize> = None;
        for (i, d) in self.drives.iter().enumerate() {
            let selected = self.selected_drive == Some(i);
            ui.add_space(6.0);
            let title = if d.label.is_empty() {
                d.letter.clone()
            } else {
                format!("{}  {}", d.letter, d.label)
            };
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(selected, RichText::new(title).strong())
                    .clicked()
                {
                    pick = Some(i);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} free", format::bytes(d.free)))
                            .small()
                            .weak(),
                    );
                });
            });
            let frac = if d.total > 0 {
                (d.used as f32 / d.total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.add(egui::ProgressBar::new(frac).desired_height(10.0));
            ui.label(
                RichText::new(format!(
                    "{} used of {}  ·  {}  ·  {}",
                    format::bytes(d.used),
                    format::bytes(d.total),
                    if d.fs.is_empty() { "?" } else { &d.fs },
                    d.type_name()
                ))
                .small()
                .weak(),
            );
        }
        if let Some(i) = pick {
            self.start_scan(i);
        }

        if let Some(i) = self.selected_drive
            && let Some(d) = self.drives.get(i)
            && let Some(s) = &self.session
        {
            let root = {
                let m = s.shared.model.lock().unwrap_or_else(|e| e.into_inner());
                (m.node(m.root).size, m.root_ready())
            };
            if root.1 {
                let gap = d.used.saturating_sub(root.0);
                ui.add_space(12.0);
                ui.separator();
                ui.label(RichText::new("Scanned total").strong());
                ui.label(format!(
                    "{} of {} used",
                    format::bytes(root.0),
                    format::bytes(d.used)
                ));
                ui.label(
                    RichText::new(format!(
                        "{} not readable without elevation (system files, other profiles, shadow copies)",
                        format::bytes(gap)
                    ))
                    .small()
                    .weak(),
                );
            }
        }
    }

    fn status_ui(&mut self, ui: &mut egui::Ui) {
        let Some(s) = &self.session else {
            ui.label(RichText::new("No scan yet").weak());
            return;
        };
        let p = &s.shared.progress;
        let done = s.shared.done.load(Ordering::Relaxed);
        let mut cancel = false;
        ui.horizontal(|ui| {
            if done {
                ui.label(RichText::new("Scan complete").strong());
            } else {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label("Scanning…");
            }
            ui.separator();
            ui.label(format!(
                "{} files",
                format::count(p.files.load(Ordering::Relaxed))
            ));
            ui.label(format!(
                "{} folders",
                format::count(p.dirs.load(Ordering::Relaxed))
            ));
            ui.label(format!(
                "{} read",
                format::bytes(p.bytes.load(Ordering::Relaxed))
            ));
            let errs = p.errors.load(Ordering::Relaxed);
            if errs > 0 {
                ui.colored_label(
                    Color32::from_rgb(230, 180, 60),
                    format!("{} unreadable", format::count(errs)),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !done && ui.button("Cancel").clicked() {
                    cancel = true;
                }
                ui.label(format!(
                    "{:.1}s",
                    s.elapsed
                        .unwrap_or_else(|| s.started.elapsed())
                        .as_secs_f64()
                ));
            });
        });
        if cancel {
            s.shared.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui) {
        let Some(shared) = self.session.as_ref().map(|s| Arc::clone(&s.shared)) else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(
                        "Select a drive on the left to scan it.\nEverything is read-only.",
                    )
                    .weak(),
                );
            });
            return;
        };

        if self.show_files {
            self.ensure_files_loaded(&shared);
        }

        let done = shared.done.load(Ordering::Relaxed);
        let version = shared
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .version;
        let stale = version != self.rows_key.0 || self.ui_epoch != self.rows_key.1;
        if stale && (done || self.last_rebuild.elapsed() >= Duration::from_millis(60)) {
            self.rebuild_rows();
        }

        let avail = ui.available_width();
        let size_cols = if self.on_disk { 2.0 } else { 1.0 };
        // The numeric columns own a fixed slice on the right and the name column
        // absorbs the rest, so the table never spills past the panel edge.
        // Indentation lives *inside* the name cell: if it were added before the
        // chevron, every deeper row would push the numbers further right.
        let fixed = CHEV_W + size_cols * SIZE_W + COUNT_W + PCT_W + GAP * 4.0;
        let name_w = (avail - fixed).max(160.0);

        // Header
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            ui.add_space(GAP + CHEV_W);
            ui.allocate_ui_with_layout(
                egui::vec2(name_w, ROW_H),
                Layout::left_to_right(Align::Center),
                |ui| {
                    // Without this the child ui shrinks to the text width and the
                    // numeric columns slide left.
                    ui.set_min_width(name_w);
                    ui.label(RichText::new("Name").strong());
                },
            );
            ui.add_space(GAP);
            col_right(ui, "Size", SIZE_W);
            if self.on_disk {
                ui.add_space(GAP);
                col_right(ui, "On disk", SIZE_W);
            }
            ui.add_space(GAP);
            col_right(ui, "Files", COUNT_W);
            ui.add_space(GAP);
            col_right(ui, "% parent", PCT_W);
        });
        ui.separator();

        let on_disk = self.on_disk;
        let rows = &self.rows;
        let mut toggle: Option<NodeId> = None;
        let mut select: Option<NodeId> = None;
        let mut copy_req: Option<NodeId> = None;
        let mut reveal_req: Option<(NodeId, bool)> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                for i in range {
                    let row = &rows[i];
                    ui.horizontal(|ui| {
                        ui.add_space(GAP);
                        if row.is_dir && row.has_children {
                            // ASCII glyphs: guaranteed to exist in the bundled font.
                            let glyph = if row.expanded { "-" } else { "+" };
                            if ui
                                .add_sized(
                                    [CHEV_W, ROW_H],
                                    egui::Button::new(RichText::new(glyph).size(13.0).strong())
                                        .frame(false),
                                )
                                .clicked()
                            {
                                toggle = Some(row.id);
                            }
                        } else {
                            ui.add_space(CHEV_W);
                        }

                        let selected = self.selected_node == Some(row.id);
                        let mut text = RichText::new(&*row.name);
                        if row.is_dir {
                            text = text.strong();
                        }
                        if row.errors > 0 {
                            text = text.color(Color32::from_rgb(230, 110, 110));
                        }
                        let resp = ui
                            .allocate_ui_with_layout(
                                egui::vec2(name_w, ROW_H),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_min_width(name_w);
                                    ui.add_space(row.depth as f32 * INDENT);
                                    ui.add(
                                        egui::Label::new(text)
                                            .sense(Sense::click())
                                            .truncate()
                                            .selectable(selected),
                                    )
                                },
                            )
                            .inner;
                        if resp.clicked() {
                            select = Some(row.id);
                        }
                        if resp.double_clicked() && row.is_dir {
                            toggle = Some(row.id);
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Copy path").clicked() {
                                copy_req = Some(row.id);
                                ui.close();
                            }
                            let label = if row.is_dir {
                                "Open in Explorer"
                            } else {
                                "Reveal in Explorer"
                            };
                            if ui.button(label).clicked() {
                                reveal_req = Some((row.id, row.is_dir));
                                ui.close();
                            }
                        });
                        if row.reparse {
                            ui.label(RichText::new("↪").weak());
                        }
                        ui.add_space(GAP);
                        col_right(ui, &format::bytes(row.size), SIZE_W);
                        if on_disk {
                            ui.add_space(GAP);
                            col_right(ui, &format::bytes(row.on_disk), SIZE_W);
                        }
                        ui.add_space(GAP);
                        col_right(ui, &format::count(row.files), COUNT_W);
                        ui.add_space(GAP);
                        col_right(ui, &format!("{:.1}%", row.pct * 100.0), PCT_W);
                    });
                }
            });

        if let Some(id) = toggle {
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.ui_epoch = self.ui_epoch.wrapping_add(1);
        }
        if let Some(id) = select {
            self.selected_node = Some(id);
        }
        if let Some(id) = copy_req {
            let path = {
                let m = shared.model.lock().unwrap_or_else(|e| e.into_inner());
                m.path_of(id)
            };
            ui.ctx().copy_text(path);
        }
        if let Some((id, is_dir)) = reveal_req {
            let path = {
                let m = shared.model.lock().unwrap_or_else(|e| e.into_inner());
                m.path_of(id)
            };
            let mut cmd = std::process::Command::new("explorer.exe");
            if is_dir {
                cmd.arg(&path);
            } else {
                cmd.arg(format!("/select,{path}"));
            }
            let _ = cmd.spawn();
        }
        if self.scanning() || shared.lazy_loads.load(Ordering::Relaxed) > 0 {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Starts a background load for every expanded directory whose file children
    /// are not in the model yet. Each node is claimed under the model lock first,
    /// so a slow load is never started twice for the same directory.
    fn ensure_files_loaded(&mut self, shared: &Arc<Shared>) {
        let on_disk = self.session.as_ref().map(|s| s.on_disk).unwrap_or(false);
        let ids: Vec<NodeId> = self.expanded.iter().copied().collect();
        let mut work: Vec<(NodeId, Vec<u16>)> = Vec::new();
        {
            let mut m = shared.model.lock().unwrap_or_else(|e| e.into_inner());
            for id in ids {
                if m.claim_file_listing(id) {
                    work.push((id, scan_dir_wide(&m.path_of(id))));
                }
            }
        }
        for (id, dir) in work {
            shared.lazy_loads.fetch_add(1, Ordering::Relaxed);
            let sh = Arc::clone(shared);
            std::thread::spawn(move || {
                load_files(&sh, id, dir, on_disk);
                sh.lazy_loads.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }
}

/// `C:\Windows` -> `\\?\C:\Windows\` as UTF-16, ready for `dir + '*'`.
fn scan_dir_wide(path: &str) -> Vec<u16> {
    let mut s = String::with_capacity(path.len() + 8);
    if !path.starts_with("\\\\?\\") {
        s.push_str("\\\\?\\");
    }
    s.push_str(path);
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s.encode_utf16().collect()
}

/// Reads one directory again and inserts its file rows. Read-only; failures are
/// silent because the directory's error count is already shown on the folder row.
fn load_files(shared: &Arc<Shared>, id: NodeId, dir: Vec<u16>, on_disk: bool) {
    let mut pattern = Vec::with_capacity(dir.len() + 1);
    pattern.extend_from_slice(&dir);
    pattern.push(u16::from(b'*'));
    let Ok(entries) = platform::enumerate_dir(&pattern) else {
        return;
    };

    let base = dir.len();
    let mut path = dir;
    let mut files = Vec::with_capacity(entries.len());
    for e in entries {
        if e.is_dir {
            continue;
        }
        let od = if on_disk {
            path.truncate(base);
            path.extend(e.name.encode_utf16());
            platform::compressed_size(&path).unwrap_or(0)
        } else {
            0
        };
        files.push((Arc::from(&*e.name), e.size, od));
    }
    let mut m = shared.model.lock().unwrap_or_else(|e| e.into_inner());
    m.insert_file_children(id, files);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        egui::Panel::top("top").show(ui, |ui| self.top_ui(ui));
        egui::Panel::left("drives")
            .resizable(true)
            .default_size(280.0)
            .min_size(220.0)
            .max_size(420.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.drives_ui(ui));
            });
        egui::Panel::bottom("status").show(ui, |ui| self.status_ui(ui));
        egui::CentralPanel::default().show(ui, |ui| self.tree_ui(ui));
    }
}

fn col_right(ui: &mut egui::Ui, text: &str, width: f32) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, ROW_H),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.label(RichText::new(text).monospace().size(12.0));
        },
    );
}

fn build_rows(m: &Model, expanded: &HashSet<NodeId>, show_files: bool) -> (Vec<Row>, u64) {
    let mut rows = Vec::new();
    let root = m.root;
    let root_node = m.node(root);
    let mut stack: Vec<(NodeId, u16, u64)> = Vec::new();
    for &c in root_node.children.iter().rev() {
        stack.push((c, 0, root_node.size));
    }
    while let Some((id, depth, parent_size)) = stack.pop() {
        let n = m.node(id);
        if !n.is_dir && !show_files {
            continue;
        }
        let is_open = n.is_dir && expanded.contains(&id);
        rows.push(Row {
            id,
            depth,
            name: n.name.clone(),
            size: n.size,
            on_disk: n.on_disk,
            files: n.files,
            // Clamped: while a scan is running the parent total is still partial,
            // so a child can temporarily be larger than the parent.
            pct: if parent_size > 0 {
                (n.size as f32 / parent_size as f32).clamp(0.0, 1.0)
            } else {
                0.0
            },
            is_dir: n.is_dir,
            expanded: is_open,
            // A directory that only contains files still needs an expander so the
            // lazily loaded file rows can be revealed.
            has_children: !n.children.is_empty() || (n.is_dir && n.files > 0),
            reparse: n.reparse,
            errors: n.errors,
        });
        if is_open {
            for &c in n.children.iter().rev() {
                stack.push((c, depth + 1, n.size));
            }
        }
    }
    (rows, m.version)
}
