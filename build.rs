//! Embeds the Windows VERSIONINFO resource and the application icon.
//!
//! The version numbers are taken from `Cargo.toml` at build time, so the exe's
//! file/product version can never drift from the crate version. The resource
//! script is generated into `OUT_DIR` and compiled with `rc.exe` on the MSVC
//! target (what Visual Studio itself uses) or `windres` on the GNU target;
//! `WINDRES` / `RC` override the executable names.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=res/app.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-env-changed=WINDRES");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");

    // "0.1.0" -> FILEVERSION 0,1,0,0 (the fourth slot stays 0 for normal releases)
    let mut parts = [0u32; 4];
    for (slot, part) in parts.iter_mut().zip(version.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    let [major, minor, patch, build] = parts;

    let icon = manifest.join("res").join("app.ico");
    let icon = icon.display().to_string().replace('\\', "/");

    let rc = format!(
        r#"1 VERSIONINFO
FILEVERSION {major},{minor},{patch},{build}
PRODUCTVERSION {major},{minor},{patch},{build}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "DefinitlyEvil"
      VALUE "FileDescription", "Disk Usage Inspector"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "disk-usage-inspector"
      VALUE "LegalCopyright", "Copyright (c) 2026 DefinitlyEvil. Apache License 2.0."
      VALUE "OriginalFilename", "disk-usage-inspector.exe"
      VALUE "ProductName", "Disk Usage Inspector"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
1 ICON "{icon}"
"#
    );

    let rc_path = out_dir.join("version.rc");
    std::fs::write(&rc_path, rc).expect("write version.rc");

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // MSVC: `rc.exe` ships with the Visual Studio + Windows SDK install that the
    // linker already needs, so it is preferred over requiring MSYS2 just for a
    // 4 KB resource block. `windres` stays the fallback (and the only option for
    // the GNU target).
    if target_env == "msvc"
        && let Some(res) = compile_with_rc(&rc_path, &out_dir)
    {
        println!("cargo:rustc-link-arg={}", res.display());
        return;
    }

    // `-O coff` is required: windres otherwise emits the binary .res format when
    // the output name ends in `.res`, and the linker rejects that. We also name
    // the output `.o` so an accidental format change fails loudly.
    let res_path = out_dir.join("version.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "windres".to_string());
    let status = Command::new(&windres)
        .arg("--input")
        .arg(&rc_path)
        .arg("--output")
        .arg(&res_path)
        .arg("-O")
        .arg("coff")
        .status();

    match status {
        Ok(s) if s.success() => {
            // Note: a manifest path containing spaces would break this; the
            // release builds run from paths without spaces.
            println!("cargo:rustc-link-arg={}", res_path.display());
        }
        Ok(s) => panic!("{windres} failed ({s}); install mingw-w64 binutils or set WINDRES"),
        Err(e) => panic!("could not run {windres}: {e}; install mingw-w64 binutils or set WINDRES"),
    }
}

/// Compiles `version.rc` with `rc.exe` and returns the `.res` file to link, or
/// `None` when no usable `rc.exe` is around (the caller then tries `windres`).
fn compile_with_rc(rc_path: &Path, out_dir: &Path) -> Option<PathBuf> {
    let res_path = out_dir.join("version.res");
    let rc_tool = std::env::var("RC").unwrap_or_else(|_| "rc.exe".to_string());
    let status = Command::new(&rc_tool)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(rc_path)
        .status();

    match status {
        Ok(s) if s.success() && res_path.is_file() => Some(res_path),
        Ok(s) => {
            println!("cargo:warning={rc_tool} exited with {s}; falling back to windres");
            None
        }
        Err(e) => {
            println!("cargo:warning=could not run {rc_tool}: {e}; falling back to windres");
            None
        }
    }
}
