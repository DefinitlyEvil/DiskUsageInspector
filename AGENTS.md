# disk-usage-inspector — AGENTS.md

**Status: IMPLEMENTED — v1 builds, scans and renders.** `cargo build --release`
produces `target\release\disk-usage-inspector.exe`; a headless harness
`target\release\duiscan.exe` measures scanner throughput. This file remains the
single source of truth for (a) the build environment, (b) the design and
(c) what the code actually does today (§6.1).

Last verified: 2026-10-24 (every command below was run on this machine; the
performance numbers in §6.3 are measured, not estimated). The release pipeline,
the exe's VERSIONINFO/icon resource and the Apache-2.0 license landed the same
day; see §2.5 and §2.6.

---

## 1. Product goal

A **read-only** Windows desktop GUI that answers "what is using my disk?":

- List every drive/volume with total, used and free space.
- Drill into any drive as an expandable folder tree.
- Show the **size of each folder** (aggregated over its whole subtree) and each file.
- Sort, refresh, and watch a scan progress; cancel at any time.

### Hard constraints (never trade these away)

1. **Read-only.** The app must never delete, move, rename, truncate, write, or
   change attributes/ACLs of anything it inspects. No "cleanup", no "delete",
   no "empty recycle bin", no "compress".
2. **No write handles on scanned paths.** Open files/dirs only with read/query
   access (`GENERIC_READ` never; prefer `FILE_READ_ATTRIBUTES`,
   `FILE_LIST_DIRECTORY`, `FILE_TRAVERSE`). Never `GENERIC_WRITE`,
   `FILE_WRITE_DATA`, `DELETE`, `WRITE_DAC`, `WRITE_OWNER`.
3. **No elevation requirement** for normal operation. An optional admin-only
   fast path (§5.2) may exist later, but must degrade gracefully.
4. **No network at runtime.** No telemetry, no update checks, no web assets.
5. **Never follow reparse points** (junctions/symlinks) by default — cycles and
   double counting.
6. A scan may fail on individual entries; it must never abort the whole scan and
   must never be retried with wider privileges.

---

## 2. Environment (this machine, Windows)

### 2.1 Activation — mandatory for every build/test command

All Rust/MSYS2 work happens **inside the environment activated by
`E:\msys64\gccenv.bat`**. From PowerShell or the agent shell, wrap commands:

```bat
cmd /c "E:\msys64\gccenv.bat && cargo build"
```

```powershell
cmd /c "E:\msys64\gccenv.bat && cargo test 2>&1"
```

`gccenv.bat` currently sets:

| Variable / action | Value | Why it matters |
|---|---|---|
| `PATH` prepend | `E:\msys64\ucrt64\bin` | `gcc` is the linker for the GNU target |
| `GOROOT` | `D:\Programs\go` | unrelated to this project |
| `GOPATH` | `E:\Cache\go` | unrelated to this project |
| `CARGO_HOME` | `E:\Cache\CargoHome` | cargo/rustup binaries + cargo config live here |
| `RUSTUP_DIST_SERVER` | `https://mirrors.tuna.tsinghua.edu.cn/rustup` | rustup mirror (must be set *before* any `rustup` call) |
| `CALL ...\VsDevCmd.bat` | VS 2022 dev env | only needed if the MSVC target is ever used |

### 2.2 Verified toolchain facts (2026-10-24)

| Item | Value |
|---|---|
| rustup | **MSYS2 UCRT64 package** (pacman), v1.29.2 — `E:\msys64\ucrt64\bin\rustup.exe` |
| default host | `x86_64-pc-windows-gnu` |
| toolchain | `stable-x86_64-pc-windows-gnu` (installed via `rustup-init` from the TUNA mirror), rustc **1.99.1** (2026-10-13), LLVM 23.1.1 |
| cargo | 1.99.1 |
| rustup home | `%USERPROFILE%\.rustup` (default, on purpose) |
| gcc | MSYS2 UCRT64 **16.2.0** (x86_64-w64-mingw32) |
| Windows SDK 10 | **not installed** |
| VS 2022 | `E:\Programs\VS2022Community` (MSVC 14.42 toolset, no SDK) |
| CPUs / RAM | Ryzen 9 9950X (16C/32T) / 125.6 GB |

**Why GNU and not MSVC:** there is no Windows 10 SDK on this machine, so the
MSVC target cannot link Win32 GUI apps without a multi-GB SDK install. The GNU
target was verified to compile and link a real Win32 GUI binary (FFI to
`user32!MessageBoxW`, `-mwindows`) using the UCRT64 gcc. Stay on
`x86_64-pc-windows-gnu` unless the SDK is installed later.

**Do not** invoke `E:\msys64\ucrt64\bin\gcc.exe` directly without activation:
`cc1.exe` fails silently with exit code 1 because its DLLs are only found when
`ucrt64\bin` is on `PATH`. Always go through `gccenv.bat`.

### 2.3 rustup rules

- **The MSYS2 `rustup` (`E:\msys64\ucrt64\bin\rustup.exe`) is the only manager.**
  It was already installed by pacman; the copy that `rustup-init` placed in
  `E:\Cache\CargoHome\bin` was retired on 2026-10-24 (renamed to
  `rustup.exe.duplicate-msys2`). Do not install a second rustup.
- `E:\Cache\CargoHome\bin` still holds the `cargo`/`rustc`/`rustfmt` proxies and
  it is the only Rust entry in the persisted user PATH; those proxies keep
  working in a plain shell. Do not delete that directory.
- rustup is pacman-managed, so `rustup self update` / `self uninstall` are
  disabled. Update the manager with `pacman -Syu` inside MSYS2, and toolchains
  with `rustup update`.
- The mirror variable comes from `gccenv.bat`; a rustup call made outside that
  environment will try the official servers.
- Useful: `rustup toolchain list`, `rustup component add clippy rustfmt`,
  `rustup target list --installed`, `rustup update`.
- Keep `RUSTUP_HOME` at its default (`%USERPROFILE%\.rustup`). `E:` has only
  ~25 GB free; toolchains live on `C:` (~43 GB free) by design.

### 2.4 crates.io mirror

- Cargo reads its config from `%CARGO_HOME%\config.toml`
  = **`E:\Cache\CargoHome\config.toml`** (USTC sparse index, created 2026-10-24).
- `%USERPROFILE%\.cargo\config.toml` also exists but is **ignored whenever
  `CARGO_HOME` is set** — do not "fix" the mirror by editing that file.
- A correctly configured build prints `Updating `ustc` index` and
  `Downloaded <crate> (registry `ustc`)`. If you see `crates.io` instead,
  `CARGO_HOME` was not set (i.e. `gccenv.bat` was not activated).
- Do not commit or hard-code mirror URLs in the project; they are machine config.

### 2.5 Repository state

- Path: `D:\Develop\rust\disk-usage-inspector`.
- Git repository: initialized on branch `main`, remote `origin` =
  `git@github.com:DefinitlyEvil/DiskUsageInspector.git`. The commit identity is
  set **repo-locally** (`git config --local user.name/user.email`), never
  globally; do not change it to a global setting.
  - `user.name` = `DefinitlyEvil`.
  - `user.email` = `8604063+DefinitlyEvil@users.noreply.github.com`. The account's
    real address is marked private on GitHub, and GitHub's GH007 rule rejects any
    pushed commit that exposes it, so commits use the noreply alias (they still
    attribute to the account). Keep the real address out of this file and out of
    the repository.
- `.gitignore` excludes `/target` and `**/*.rs.bk`.
- Build output is pinned to `./target` by `.cargo/config.toml`
  (`[build] target-dir = "target"`), so `target\release\...` is always the
  artifact location regardless of the shell's working directory.
- Screenshots may be committed **only** when they show a synthetic fixture: no
  personal folder/file names, no user paths, no real document titles. The
  README screenshot (`screenshot.png`) was captured against a `subst`-mapped
  fixture volume with made-up project names. Benchmark logs and scratch captures
  stay out of the repo.
- **License:** Apache-2.0 (`LICENSE` + `NOTICE`). `Cargo.toml` declares
  `license = "Apache-2.0"`; when a dependency with different terms is added,
  record it in `NOTICE` in the same commit.
- **Release pipeline:** `.github/workflows/release.yml` runs on a `v*` tag (or
  `workflow_dispatch` with an existing tag), on `windows-latest` with MSYS2
  UCRT64 + `stable-x86_64-pc-windows-gnu`. It re-runs the §3 gate, enforces the
  §11.2 read-only grep, checks the exe's VERSIONINFO against the tag, stages
  `dist/disk-usage-inspector-<tag>-win64.zip` (both exes, docs, `SHA256SUMS.txt`),
  attests build provenance and publishes the GitHub release. `.github/workflows/ci.yml`
  runs the same gate on every push and pull request.
- **Signing:** releases are **unsigned** for now. Free signing only — the
  SignPath Foundation OSS program is applied for; no paid certificates. Policy,
  roles and verification steps live in `CODE_SIGNING.md`. Never commit a
  certificate, PFX or private key.
- **Release tagging:** bump `Cargo.toml` version, commit, then
  `git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin vX.Y.Z`. A mismatch between
  the tag and the exe's `FileVersion` fails the workflow before publishing.

---

## 3. Commands

| Task | Command |
|---|---|
| Build (debug) | `cmd /c "E:\msys64\gccenv.bat && cargo build"` |
| Run | `cmd /c "E:\msys64\gccenv.bat && cargo run"` |
| Build release | `cmd /c "E:\msys64\gccenv.bat && cargo build --release"` |
| Test | `cmd /c "E:\msys64\gccenv.bat && cargo test"` |
| Lint (must pass clean) | `cmd /c "E:\msys64\gccenv.bat && cargo clippy --all-targets -- -D warnings"` |
| Format check | `cmd /c "E:\msys64\gccenv.bat && cargo fmt --check"` |
| Scanner benchmark | `cmd /c "E:\msys64\gccenv.bat && cargo run --release --bin duiscan -- C:\ --top 10"` |
| Regenerate the icon (design changes only) | `pwsh -NoProfile -File tools\make-icon.ps1` |
| Check the exe's embedded metadata | `(Get-Item target\release\disk-usage-inspector.exe).VersionInfo` |
| Cut a release | `git tag -a v0.1.0 -m "v0.1.0"` then `git push origin v0.1.0` |

Artifacts:
- `target\release\disk-usage-inspector.exe` — the GUI (single self-contained
  exe; UCRT ships with Windows 10+, no extra runtime DLLs).
- `target\release\duiscan.exe` — console harness, same scanner without egui.
  It is a **console** subsystem binary, so it can be run and measured from a
  script; the GUI binary is GUI subsystem, so `& .\target\release\...exe` returns
  immediately and its stdout is lost.

`duiscan` flags: `duiscan <path> [--on-disk] [--no-files] [--threads N] [--top N]`.
It prints elapsed time, file/folder counts, logical (and optional on-disk) bytes,
node count, unreadable count, throughput, and the top entries. It also streams a
2-second progress line to stderr and warns after 10 s without progress, which is
how stalls (or a deadlock) are detected without a debugger.

---

## 4. Scope of v1

### In scope
- Drive list: letter/label/filesystem/type/total/used/free; skip empty optical
  drives gracefully; show network drives as "not scanned" unless clicked.
- Scan a chosen volume: full recursive enumeration in the background.
- Tree view: expand/collapse any directory; virtualized rendering (only visible
  rows are drawn); children sorted by size descending by default.
- Per-folder aggregate size, recursive file count, direct file count.
- **Size metric switch** (decided 2026-10-24): logical bytes by default, plus an
  optional on-disk/allocated column (compressed + sparse aware). The metric is
  chosen *before* a scan; switching it after a scan requires a rescan (logical
  data is always collected, allocation data only when the toggle is on).
- Progress: bytes/files seen, current path, elapsed, cancel.
- Filters: hide files / show files, name filter.
- Read-only extras: copy path, "open in Explorer", reveal item (all
  non-mutating shell operations).

### Explicit non-goals
- Deleting, moving, compressing, deduplicating, cleaning.
- Live filesystem monitoring / background daemon.
- Disk health/SMART, partition editing, formatting.
- Charts/pie maps (possible later, not v1).
- Cross-platform support (Windows only; the domain logic may stay portable).

### UX acceptance criteria
- Cold scan of `C:` (≈1M files) completes in well under 2 minutes warm-cache.
- UI stays responsive (≥ 60 fps scrolling) during scanning of 100k+ rows.
- Cancel stops within ~200 ms.
- Unreadable directories are counted and visible, never fatal.
- No operation in the UI can alter the scanned volume.

---

## 5. Domain study — how Windows reports sizes

### 5.1 Volumes

- Enumerate with `GetLogicalDriveStringsW`, classify with `GetDriveTypeW`
  (`DRIVE_FIXED`, `DRIVE_REMOVABLE`, `DRIVE_REMOTE`, `DRIVE_CDROM`, `DRIVE_RAMDISK`).
- Capacity with `GetDiskFreeSpaceExW` (free bytes available to caller, total,
  total free). **Used = total − total free.**
- Labels/FS with `GetVolumeInformationW` (volume label, filesystem name, serial,
  max component length, flags).
- Mounted volumes without a drive letter: `FindFirstVolumeW` /
  `GetVolumePathNamesForVolumeNameW`. v1 may list only lettered drives and
  document the gap; the drive list must not crash on them.
- **Expectation setting (show in UI):** the sum of folder sizes will be *less*
  than the drive's used bytes. Pagefile/hiberfil, `$Recycle.Bin`, System Volume
  Information (shadow copies), files locked to SYSTEM, and other users' profiles
  are either invisible or unreadable without admin. This is a feature of the
  domain, not a bug — label it.

### 5.2 Directory enumeration options

| Approach | Speed | Notes |
|---|---|---|
| `std::fs::read_dir` | okay | simple, `OsString` paths, per-entry metadata syscall |
| `FindFirstFileExW` + `FindExInfoBasic` + `FIND_FIRST_EX_LARGE_FETCH` | **fast** | batched results, size + attributes in one call; **recommended** |
| `NtQueryDirectoryFile` with `FileIdBothDirectoryInfo` | fastest user-mode | gives file IDs (hard-link dedupe); more unsafe/FFI |
| MFT / USN journal (`FSCTL_ENUM_USN_DATA`) | whole volume in seconds | **requires admin**, NTFS only, complex; out of scope for v1, optional later |
| crate `jwalk` (rayon walker) | fast | easy parallelism, but coarse control over cancellation/errors/reparse and no size-on-disk |

Decision: custom scanner over `FindFirstFileExW`, one directory handle at a
time, parallel across directories (§5.5). This keeps cancellation, error
policy, reparse handling and future NTFS fast-path under our control.

### 5.3 Size semantics

- **Logical size** (default): `nFileSizeHigh/Low` from the find data. Cheap.
- **Size on disk** (optional toggle): `GetCompressedFileSizeW` per file (honours
  compression and sparse files) or `GetFileInformationByHandleEx(FileStandardInfo)`
  `AllocationSize`. Noticeably slower (one syscall per file); must be opt-in and
  clearly labelled.
- **Hard links** are counted once per name by default → a file with N links is
  counted N times. Deduping requires file IDs (per-file handle or
  `FileIdBothDirectoryInfo`) and is a later optimisation. Document it.
- **Compressed/sparse** files: logical size is the honest "how big is this file",
  on-disk is the honest "what does it cost". Offer both, never mix them in one column.
- **Reparse points** (junctions, symlinks, OneDrive placeholders): do not recurse
  into them; count the link itself (usually 0) and mark the row with a badge.
  Guard hard against cycles anyway (max depth, in-flight path set).
- **Long paths (> 260)**: use the `\\?\` prefix for all Win32 calls, and/or ship
  a `longPathAware` manifest. Never `MAX_PATH`-truncate.
- **Integers**: `u64` bytes everywhere; saturating adds; format with IEC units
  (KiB/MiB/GiB/TiB) plus an exact byte tooltip.

### 5.4 Error classes

| Error | Handling |
|---|---|
| Access denied (`ERROR_ACCESS_DENIED`) | count on the node, keep walking siblings |
| Reparse/symlink | do not follow, mark node, continue |
| Path too long | `\\?\` prefix should prevent; count if it still happens |
| Vanished during scan (`ERROR_FILE_NOT_FOUND`) | ignore, it is a snapshot |
| Drive removed / not ready | abort that volume's scan with a clear message |
| Sharing violation on open | fall back to size-on-disk unavailable, keep logical size |

### 5.5 Concurrency model

- Fixed worker pool (`min(8, logical CPUs)` default; 32 threads on this box is
  counter-productive for a single spindle/NVMe queue and hurts UI latency).
- Work queue of directory tasks; a worker enumerates one directory, appends
  child dirs to the queue, and accumulates file sizes locally.
- **Do not** spawn a thread/task per directory (explosion, memory, syscall storm).
- Cancellation: `AtomicBool` checked in the per-entry loop (every ~1024 entries)
  so cancel is sub-200 ms.
- Progress: atomics for files/bytes/dirs; UI polls at 10–20 Hz. Never send an
  event per file across a channel.
- Node completion: each dir tracks `children_pending`; when it reaches 0, size
  is finalized bottom-up and a batched update is sent to the UI.

### 5.6 Data model & UI rendering

- Arena: `Vec<Node>`, `NodeId = u32`, parent/child indices; avoids `Rc<RefCell>`
  webs and makes the tree cheap to sort and to send across threads.
- Each node: name, parent, first_child/next_sibling (or child range), flags
  (dir/file/reparse/error), logical size, on-disk size (optional), counts,
  state (pending/scanning/done).
- UI keeps a **flattened visible-row list** (rebuilt when expansion or sort
  changes) and draws only rows intersecting the viewport. 100k+ rows must be
  smooth.
- Sorting: children sorted by aggregate size desc; ties by name. Sort after a
  directory completes, not on every update.
- Columns: name (indented, expander triangle), size, % of parent, items, state.
- Expand triggers nothing lazily in v1 (the whole volume is scanned up front);
  lazy per-drive scanning can come later if startup latency matters.

---

## 6. Architecture as built

### 6.1 Module map

```
src/
  main.rs            # eframe entry; windows_subsystem = "windows" in release
  lib.rs             # pub mod app/format/model/platform/scan
  app.rs             # eframe App: panels, drive list, tree rows, status bar
  model.rs           # Node arena, aggregation, sorting, path_of, lazy file insert
  scan.rs            # worker pool over a dir queue, cancellation, progress, Shared
  platform.rs        # all unsafe Win32 FFI: volumes + directory enumeration
  format.rs          # IEC byte formatting + compact counts
  bin/duiscan.rs     # console harness (same scanner, no egui)
  build.rs           # embeds VERSIONINFO + icon, compiled by windres
tests/scan_smoke.rs  # fixture totals, cancellation, drive enumeration
examples/dump.rs     # debug tree printer
```

Build-time files outside `src/`:

```
res/app.ico          # application icon (16-256 px); regenerated by tools/make-icon.ps1
.github/workflows/   # ci.yml (gate on push/PR) + release.yml (tag -> zip + release)
```

Data model (`model.rs`), one `Vec<Node>`, `NodeId = u32`:

```rust
Node { parent, name: Arc<str>, size, on_disk, files, pending, enumerated,
       is_dir, reparse, state, files_listed, errors, children: Vec<NodeId> }
```

- Aggregation is bottom-up: a directory accumulates its own files immediately and
  its children's totals as they finish; `finish_dir` finalises only when
  `enumerated && pending == 0`. `propagate` walks parents adding deltas, so a
  directory's size is correct the moment its last child completes — this is why
  the tree shows growing numbers during a scan instead of blanks.
- `errors` counts unreadable entries per directory subtree; a failure never
  aborts a scan.
- **Lazy file rows.** The scan stores *directory* nodes only (`include_files:
  false`). When "Show files" is enabled, each visible directory whose files have
  not been listed yet is claimed exactly once (`claim_file_listing`, guarded by a
  `files_listed` flag under the model lock) and handed to a background thread
  that enumerates that one level and calls `insert_file_children`. That call adds
  nodes **without touching any aggregate**, because the bytes/files were already
  counted during the scan — so totals never double count and the previously
  displayed numbers stay stable. The UI repaints while `Shared::lazy_loads > 0`.
- Rationale: on C: that keeps the arena at ~500k directory nodes / ~109 MB peak
  instead of 2.8M nodes / ~448 MB (measured, §6.3).

Scanner (`scan.rs`):

- `Shared { model: Mutex<Model>, progress, cancel: AtomicBool, done,
  lazy_loads: AtomicUsize, opts, queue: Mutex<VecDeque<Task>>, cv: Condvar }`;
  workers pop from the queue, sleep on the condvar when it is empty, and exit
  when `queue.is_empty() && in_flight == 0`.
- `default_threads() = available_parallelism().clamp(4, 12)` — measured optimum on
  this box; more threads only add contention.
- Cancellation is checked per directory batch, so cancel latency is one
  directory enumeration (milliseconds).
- Progress is a handful of atomics polled by the UI at ~16 Hz; no per-file events.

UI (`app.rs`):

- `egui::Panel::top(bottom/left)` + `CentralPanel` (the 0.37 API; there is no
  `TopBottomPanel`/`SidePanel` in this version — see §6.2).
- Tree rows are a flat `Vec<Row>` rebuilt from the arena only when
  `(model.version, ui_epoch)` changes and at most every 60 ms during a scan;
  `ScrollArea::show_rows` draws only the visible slice.
- Column layout uses fixed widths (`CHEV_W`, `SIZE_W`, `COUNT_W`, `PCT_W`, `GAP`)
  with the name column absorbing the remainder. **Trap:** `ui.allocate_ui_with_layout`
  with a requested size does *not* advance the parent cursor by that size when the
  child ui shrinks to its content — call `ui.set_min_width(w)` inside the child or
  every numeric column slides left. `add_sized` + `Label::halign` centres the
  widget, so it is not a substitute.
- **Column arithmetic (fixed 2026-10-24).** The first version added the tree
  indent *before* the chevron and relied on `item_spacing` for the gaps, so deep
  rows pushed the numbers right and `% parent` ran off the panel edge (visible in
  the first README screenshot). The table now sets `item_spacing = (0, 0)`, adds
  every gap explicitly, reserves the numeric columns from the right
  (`name_w = avail - (CHEV_W + numeric + 4*GAP)`) and draws `depth * INDENT`
  *inside* the name cell. Consequence: numeric columns never move when rows are
  expanded, and the table can never overflow the panel.
- Expander glyphs are ASCII `+`/`-`; the Unicode chevrons `▸`/`▾` are missing from
  the bundled font and render as dots.
- Drive rows show label/letter, free space, a bar of used/total, and a detail line
  with used, filesystem and type. The "Scanned total" block under the drive list
  is the honest gap between the volume's used bytes and the sum of what a
  non-elevated scan can see.
- Starting a scan clears `expanded`/`selected_node`/`rows`, because `NodeId`s are
  only meaningful inside one model.

### 6.2 FFI rules that cost us a debugging session

1. **`windows-sys` PCWSTR arguments must be NUL-terminated.** A `&[u16]` slice
   from `str::encode_utf16().collect()` is *not* terminated; passing it as
   `PCWSTR` reads past the end and `FindFirstFileExW` fails intermittently with
   error 2 (not found) or 123 (invalid name). Always `push(0)` on a private copy
   before the call — `platform::enumerate_dir` does this.
2. Build every path as `\\?\` + absolute + trailing `\` before passing to
   `FindFirstFileExW`; that also makes >260-char paths work (verified with a
   120-level, 3290-char fixture).
3. `FindExInfoBasic` + `FIND_FIRST_EX_LARGE_FETCH` returns size and attributes in
   one call; `GetCompressedFileSizeW` is only called when on-disk sizes are
   requested (one extra syscall per file).
4. `FILE_ATTRIBUTE_REPARSE_POINT` entries are reported but never recursed into
   (verified with a junction fixture: the link shows 0 B, the target is not
   counted twice).
5. In `app.rs`/`scan.rs` the code is safe Rust; `unsafe` lives only in
   `platform.rs` with a safety comment per block.

### 6.3 Measured performance (release, this machine, warm cache)

| Run | Time | Files | Folders | Bytes | Nodes | Peak WS |
|---|---|---|---|---|---|---|
| `C:` dirs only (`--no-files`, the GUI default) | **6.3–6.6 s** | 2.30 M | 500 k | 440.1 GiB | 500 k | **~109 MB** |
| `C:` with file nodes (`include_files = true`) | **7.2 s** | 2.30 M | 500 k | 440.1 GiB | 2.80 M | ~448 MB |
| `D:` GUI scan | 23.2 s | 1.2 M | 170 k | 924.3 GiB | 170 k | ~315 MB |
| `R:` RamDisk (tiny) | < 0.1 s | 4 | 2 | 217.7 MiB | 2 | ~278 MB (egui baseline) |

Throughput ≈ 350–370 k files/s on C:. Memory is dominated by egui itself
(~270 MB baseline working set once the window has rendered); the scan adds only
the arena. Unreadable entries on C: ≈ 285 (system files, other profiles, shadow
copies) and are reported, never fatal.

### 6.4 GUI framework choice (history)

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **eframe/egui** | fastest to build a custom virtualized tree; immediate mode; single exe; good DPI/text | non-native look; GPU dependency (wgpu by default, glow optional); binary ~5–8 MB | **recommended** |
| raw Win32 (`windows` crate) + `SysTreeView32` | truly native, tiny binary | tree virtualization and 100k nodes are painful; lots of unsafe; slow to iterate | fallback if native look is mandatory |
| Slint | declarative, native-ish, good docs | tree/list models + dynamic sorting need work; extra build step | no |
| Tauri | web UI, flexible | WebView2 dependency, bigger surface, overkill for a local tool | no |

Recommendation: **eframe 0.37 + egui 0.37** (versions verified on crates.io
2026-10-24). **User confirmed 2026-10-24: use the latest egui/eframe**; the
raw-Win32 row above is kept as history, not as an option to revisit.

**Phase 0 spike result — PASSED 2026-10-24:**
- `cargo add eframe` resolved 0.37.0; 425 packages, all fetched from the `ustc`
  mirror; cold debug build 1m43s; binary 5.1 MB.
- A real `eframe::App` window ran for 6 s (wgpu → DX12) and was killed cleanly
  with no stderr. The GNU toolchain builds and runs the GUI stack.
- **API trap:** in egui/eframe 0.37 the `App` trait method is
  `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)` — *not*
  `update(&mut self, ctx, frame)` as in older versions — and panels are added to
  that `Ui` (`egui::CentralPanel::default().show(ui, |ui| ...)`). Read
  docs.rs/egui/0.37 before writing UI code; most tutorials online are for ≤0.31.
- Default features pull in `wgpu`. If a target machine lacks DX12/Vulkan, switch
  to `default-features = false, features = ["glow", "default_fonts"]`.

---

## 7. Milestones

- **Phase 0 — spike.** PASSED 2026-10-24 (§6.4): eframe 0.37 builds and runs on
  the GNU toolchain, no console flash, release profile capped at `opt-level=3`,
  thin LTO, `panic="abort"`.
- **Phase 1 — drive list.** DONE: lettered volumes with label, filesystem, type,
  total/used/free, bar, refresh, rescan.
- **Phase 2 — scanner.** DONE: `FindFirstFileExW` enumerator, worker pool,
  cancel, progress, arena model, plus the `duiscan` console harness used for the
  §6.3 numbers.
- **Phase 3 — tree UI.** DONE: virtualized rows, expand/collapse, size
  formatting, indentation, `% parent`, unreadable badge, live updates during a
  scan.
- **Phase 4 — polish.** PARTIAL: status bar, filters, on-disk toggle, elapsed,
  copy-path plumbing. Still open: name filter, copy-path/reveal context menu,
  settings persistence, dark mode.
- **Phase 5 — hardening.** PARTIAL: long paths verified with a 3290-char fixture,
  junction fixture verified, cancellation test in `tests/scan_smoke.rs`. Since
  then: VERSIONINFO + icon resource, release pipeline, CI read-only grep. Still
  open: 5M-file stress run, a denied-directory fixture.

---

## 8. Key decisions (ADR summary)

| # | Decision | Rationale |
|---|---|---|
| 1 | GNU target, UCRT64 gcc linker | no Windows SDK installed; verified link of Win32 GUI |
| 2 | Custom `FindFirstFileExW` scanner | control over cancel/errors/reparse; speed |
| 3 | Arena + flattened visible rows | 100k+ node trees without per-node allocations |
| 4 | Logical size default, on-disk opt-in per scan | one syscall vs two per file; toggling mid-session needs a rescan |
| 5 | Don't follow reparse points | cycles, double counting, OneDrive recursion |
| 6 | Poll progress at 10–20 Hz | avoids UI thread flooding |
| 7 | No admin requirement | normal user can inspect own files; admin path deferred |
| 8 | GUI = latest egui/eframe (0.37 at time of writing) | user decision 2026-10-24 |
| 9 | Both size metrics in v1 (logical + on-disk toggle) | user decision 2026-10-24 |
| 10 | **Lazy file rows** — the scan stores directories only, files are enumerated one level at a time when "Show files" is on | measured 109 MB vs 448 MB peak on C:, and lazy inserts never disturb aggregate totals |
| 11 | `% parent` is clamped to 100% | while a scan runs a child can legitimately exceed its still-partial parent |
| 12 | Expander glyphs ASCII `+`/`-` | the bundled font has no `▸`/`▾` glyph; they rendered as dots |
| 13 | **Apache-2.0**, `LICENSE` + `NOTICE` | permissive, OSI-approved (required for the free signing program), no copyleft friction for a desktop tool |
| 14 | Portable zip release, tag-driven CI, no installer | zero install/uninstall surface for a single-exe tool; the tag is the release trigger and the gate runs before anything is published |
| 15 | **Free code signing only** (SignPath Foundation OSS); unsigned + checksums + attestation until accepted | paid certificates were explicitly ruled out; the signing policy and verification steps live in `CODE_SIGNING.md` |

---

## 9. Risks & mitigations

| Risk | Mitigation |
|---|---|
| GUI stack fails to build on GNU toolchain | **retired** — eframe 0.37 built and ran on 2026-10-24; MSVC + SDK install stays as fallback |
| `eframe` GPU init fails on odd drivers | eframe has a software fallback; test on the target machine early |
| Scan slower than expected on HDD/network | measure in Phase 2 CLI; offer depth limit and skip-network |
| Memory blow-up on 5M files | **measured**: dirs-only on C: is 500k nodes / ~109 MB; file nodes would be 2.8M / ~448 MB, so they are lazy by design (§6.1) |
| Antivirus flags a scanner reading every file | only read metadata (attributes/size), never file contents |
| User expects totals to equal "used space" | explicit "unaccounted" row = used − scanned |
| Long-path bugs | `\\?\` everywhere from day one, fixture paths > 260 chars |

---

## 10. Testing strategy

What exists and must stay green (`cargo test` → 9 tests, all pass):

- `format::tests` — IEC byte formatting and compact counts (boundaries, 0, TiB).
- `model::tests::aggregation` — bottom-up totals from out-of-order child
  completion.
- `model::tests::out_of_order_completion` — a directory is only finalised when
  `enumerated && pending == 0`, regardless of insertion order.
- `model::tests::path_reconstruction` — `path_of` rebuilds the full path from
  parents.
- `model::tests::lazy_file_insert_keeps_totals` — inserting file children must
  not change any aggregate (this is the invariant that makes §6.1's lazy loading
  safe).
- `tests/scan_smoke.rs::totals_match_fixture` — scans a generated temp fixture
  (nested dirs, empty dir, zero-byte files) and asserts exact totals.
- `tests/scan_smoke.rs::cancellation_is_honoured` — sets the cancel flag and
  asserts the scan stops.
- `tests/scan_smoke.rs::multiple_drives_are_enumerable` — `GetLogicalDriveStringsW`
  path works and yields at least one volume.

Manual / fixture checks done during development (keep doing them after scanner
changes):

| Fixture | Expectation | Result |
|---|---|---|
| 120 nested dirs, 3290-char path (`%TEMP%\dui-deep-fixture`) | scanned via `\\?\`, correct bytes | 1 file, 4096 B, 121 nodes |
| junction `link -> real`, 1 MiB inside (`mklink /J`) | link counted as 0 B, target not double counted | 1.0 MiB total |
| real drive C: | ~285 unreadable entries reported, scan completes | see §6.3 |
| RamDisk R: | instant scan, lazy file rows, folder expansion | verified via UI screenshot |

Still to add: a denied-directory ACL fixture (machine-dependent, keep `#[ignore]`
if not creatable), 5M-file stress run, and a smoke test for `insert_file_children`
sorting/tie-breaking.

---

## 11. Rules for agents working in this repository

1. **Read §1 constraints before writing any code.** Any patch that can delete,
   rename, truncate, or write to a scanned path is rejected outright.
2. Forbidden symbols/APIs in `src/` (CI enforces this grep in `ci.yml`/`release.yml`):
   `remove_file`, `remove_dir`, `remove_dir_all`, `rename`, `set_permissions`,
   `OpenOptions::write`, `File::create`, `CreateFileW` with `GENERIC_WRITE`/
   `DELETE`, `SHFileOperation`, `MoveFile`, `DeleteFile`, `SetFileAttributes`.
3. `unsafe` only in `src/platform/`; every unsafe block gets a safety comment.
4. `cargo fmt` + `cargo clippy -- -D warnings` clean before finishing.
5. Edition 2024, `rust-version = "1.99"`.
6. Update this file when a decision in §8 changes; keep §2.2 version facts current.
7. Do not add dependencies that phone home, spawn installers, or bundle a webview.
8. Prefer `windows-sys` (thin, no macros) unless the `windows` crate's ergonomics
   are clearly worth it; do not pull both.

---

## 12. Decisions log / remaining questions

**Answered 2026-10-24 by the user:**

1. **UI stack → latest egui/eframe.** Start on 0.37 and keep the dependency
   current; the raw-Win32 option is dropped (kept above only as history).
2. **Size metric → both.** Logical is always collected; on-disk is an extra
   column collected only when enabled before a scan.
3. **Drive scope for v1 → lettered drives only** (`GetLogicalDriveStringsW`).
   Mount-point-only volumes and network shares are not listed yet; the drive
   list tolerates them but does not enumerate them. Revisit only if asked.
4. **Output directory → `./target`** (`.cargo/config.toml`), and `target` is
   git-ignored. The deliverable is `target\release\disk-usage-inspector.exe`.
5. **License → Apache-2.0**, full text in `LICENSE`, attribution in `NOTICE`.
6. **Distribution → portable zip, no installer.** A `vX.Y.Z` tag makes GitHub
   Actions run the gate, build, package `disk-usage-inspector-<tag>-win64.zip`
   with `SHA256SUMS.txt` and build provenance, then publish the release (§2.5).
   No Start Menu entry, nothing to uninstall.
7. **Code signing → free only.** Paid certificates are ruled out; if the free
   OSS signing program declines, releases stay unsigned with checksums and
   attestation as the trust anchors (`CODE_SIGNING.md`).

**Still open (must be answered before the feature that needs them):**

8. **Admin fast path:** optional MFT/USN "instant" mode (admin, NTFS only)? Not
   needed for v1: C: is already 6.3 s without it. Decide only if a slower disk
   makes a full scan painful.
