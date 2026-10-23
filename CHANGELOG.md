# Changelog

All notable changes to Disk Usage Inspector are recorded here. Versions follow
[Semantic Versioning](https://semver.org/); the exe's `FileVersion` always matches
the release tag, and the CI job fails the release if it does not.

## 0.1.1 — 2026-10-24

**Same app, rebuilt with the MSVC toolchain.**

The 0.1.0 binary was built with the mingw (GNU) toolchain. An unsigned mingw
executable is exactly the shape antivirus heuristics distrust — the import table,
section layout and runtime strings differ from anything Visual Studio emits — so
this release switches the shipped artifact to `x86_64-pc-windows-msvc` (Visual
Studio 2022 14.42 + Windows SDK 10.0.22621). It now looks like an ordinary
Visual Studio application, which is what SmartScreen and AV heuristics expect.

Nothing else changed: same scanner, same UI, same read-only guarantees.

- The static CRT is linked in (`+crt-static`), so there is still **no runtime DLL
  dependency** — one exe, nothing to install, no `VCRUNTIME140.dll`.
- `build.rs` now compiles the VERSIONINFO/icon block with the SDK's `rc.exe` on
  MSVC and falls back to `windres`, so MSYS2 is no longer needed to build when
  Visual Studio is present.
- `rust-toolchain.toml` pins the MSVC toolchain for every `cargo` call in the
  repository; the GNU toolchain stays installed as a fallback for bisecting.
- CI and the release workflow install the MSVC toolchain; MSYS2 supplies only
  `windres`.

| | 0.1.0 (GNU) | 0.1.1 (MSVC) |
| :--- | ---: | ---: |
| exe as shipped | 11 884 032 B | 11 460 608 B |
| release zip | 5 561 948 B | 5 398 424 B |
| `C:` dirs-only, warm cache | 6.31 s | 6.31 s |

No performance or size regression; the leak scan that guards the release stayed
clean, so no new machine-specific strings arrive through `link.exe` or `rc.exe`.

## 0.1.0 — 2026-10-24

First release.

- Drive list with label, filesystem, type, total/used/free and a usage bar.
- Full recursive scan of a volume on a worker pool, with cancel and live progress.
  2.30 M files / 500 k folders on `C:` in 6.3 s warm (≈ 364 k files/s).
- Virtualized folder tree with per-folder aggregate size, `% parent`, file counts
  and an unreadable-entry badge; lazy file rows keep the arena at ~109 MB on `C:`.
- Logical size by default, on-disk/allocated size as a per-scan toggle.
- Strictly read-only: metadata-only opens, no write handles, no elevation, no
  network, reparse points never followed.
- Portable zip, Apache-2.0, `SHA256SUMS.txt` and SLSA build provenance; unsigned
  pending the free OSS signing program (`CODE_SIGNING.md`).
