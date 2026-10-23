//! All Win32 FFI lives here. Read-only by construction: this module only ever
//! queries metadata (find data, volume info, compressed size). It never opens a
//! handle with write access and never modifies an entry.

use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH, FindClose,
    FindExInfoBasic, FindFirstFileExW, FindNextFileW, GetCompressedFileSizeW, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW, WIN32_FIND_DATAW,
};

pub const DRIVE_REMOVABLE: u32 = 2;
pub const DRIVE_FIXED: u32 = 3;
pub const DRIVE_REMOTE: u32 = 4;
pub const DRIVE_CDROM: u32 = 5;
pub const DRIVE_RAMDISK: u32 = 6;

#[derive(Clone)]
pub struct DriveInfo {
    /// `C:` — display name.
    pub letter: String,
    /// `C:\` — path used for volume queries.
    pub root: String,
    /// `\\?\C:\` — prefix-safe root used for scanning.
    pub scan_root: String,
    pub label: String,
    pub fs: String,
    pub drive_type: u32,
    pub total: u64,
    pub free: u64,
    pub used: u64,
}

impl DriveInfo {
    pub fn type_name(&self) -> &'static str {
        match self.drive_type {
            DRIVE_FIXED => "Fixed",
            DRIVE_REMOVABLE => "Removable",
            DRIVE_REMOTE => "Network",
            DRIVE_CDROM => "Optical",
            DRIVE_RAMDISK => "RAM disk",
            _ => "Unknown",
        }
    }

    /// True when the volume can be enumerated in practice.
    pub fn scannable(&self) -> bool {
        self.total > 0 && self.drive_type != DRIVE_CDROM
    }
}

/// Enumerate lettered volumes with capacity. Read-only.
pub fn list_drives() -> Vec<DriveInfo> {
    let mut buf = [0u16; 512];
    // SAFETY: buf is a valid, writable buffer of the advertised length.
    let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) };
    if n == 0 {
        return Vec::new();
    }
    let n = (n as usize).min(buf.len());
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        let start = i;
        while i < n && buf[i] != 0 {
            i += 1;
        }
        if i == start {
            break;
        }
        let root_wide = &buf[start..i];
        i += 1; // skip NUL

        let root = String::from_utf16_lossy(root_wide);
        if root.len() < 3 {
            continue;
        }
        let letter = root[..2].to_string();

        // SAFETY: root_wide is NUL-terminated (slice taken before the NUL).
        let drive_type = unsafe { GetDriveTypeW(root_wide.as_ptr()) };
        if drive_type <= 1 {
            continue; // DRIVE_UNKNOWN / DRIVE_NO_ROOT_DIR
        }

        let (total, free) = volume_space(root_wide);
        let (label, fs) = volume_identity(root_wide);

        out.push(DriveInfo {
            scan_root: format!("\\\\?\\{root}"),
            letter,
            root,
            label,
            fs,
            drive_type,
            total,
            free,
            used: total.saturating_sub(free),
        });
    }
    out
}

fn volume_space(root: &[u16]) -> (u64, u64) {
    let mut free_avail = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    // SAFETY: all three out-params are valid; root is NUL-terminated.
    let ok =
        unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free_avail, &mut total, &mut total_free) };
    if ok == 0 { (0, 0) } else { (total, total_free) }
}

fn volume_identity(root: &[u16]) -> (String, String) {
    let mut label = [0u16; 261];
    let mut fs = [0u16; 261];
    let mut serial = 0u32;
    let mut max_component = 0u32;
    let mut flags = 0u32;
    // SAFETY: buffers and out-params are valid; lengths match the arrays.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            &mut max_component,
            &mut flags,
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return (String::new(), String::new());
    }
    (utf16z_to_string(&label), utf16z_to_string(&fs))
}

fn utf16z_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub struct DirEntry {
    pub name: Box<str>,
    pub is_dir: bool,
    pub reparse: bool,
    pub size: u64,
}

/// Read one directory. `pattern` is a NUL-free `\\?\...\*` path; a NUL is
/// appended here because Win32 requires a terminated string and `&[u16]` is not.
///
/// Uses `FindFirstFileExW` with `FindExInfoBasic` + `FIND_FIRST_EX_LARGE_FETCH`
/// so the kernel returns batched entries without 8.3 short names.
pub fn enumerate_dir(pattern: &[u16]) -> Result<Vec<DirEntry>, u32> {
    let mut path = Vec::with_capacity(pattern.len() + 1);
    path.extend_from_slice(pattern);
    path.push(0);

    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    // SAFETY: path is NUL-terminated, data is a valid out-param.
    let handle = unsafe {
        FindFirstFileExW(
            path.as_ptr(),
            FindExInfoBasic,
            &mut data as *mut WIN32_FIND_DATAW as *mut c_void,
            0, // FindExSearchNameMatch
            ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: no resources held.
        let err = unsafe { GetLastError() };
        // Empty directory and vanished directory are both "nothing here".
        if err == ERROR_FILE_NOT_FOUND || err == ERROR_NO_MORE_FILES {
            return Ok(Vec::new());
        }
        return Err(err);
    }

    let mut out = Vec::new();
    loop {
        let attrs = data.dwFileAttributes;
        let name_len = data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(data.cFileName.len());
        let name = String::from_utf16_lossy(&data.cFileName[..name_len]);
        if name != "." && name != ".." {
            let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
            let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
            out.push(DirEntry {
                name: name.into_boxed_str(),
                is_dir,
                reparse: attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                size,
            });
        }
        // SAFETY: handle is valid until FindClose; data is a valid out-param.
        let more = unsafe { FindNextFileW(handle, &mut data) };
        if more == 0 {
            // SAFETY: no resources held.
            let err = unsafe { GetLastError() };
            // SAFETY: handle comes from FindFirstFileExW and is no longer used.
            unsafe { FindClose(handle) };
            if err == ERROR_NO_MORE_FILES {
                return Ok(out);
            }
            if err == ERROR_FILE_NOT_FOUND {
                return Ok(out);
            }
            return Err(err);
        }
    }
}

/// Allocated size of a file (honours NTFS compression and sparse files).
/// `path` must be NUL-free.
pub fn compressed_size(path: &[u16]) -> Option<u64> {
    let mut buf = Vec::with_capacity(path.len() + 1);
    buf.extend_from_slice(path);
    buf.push(0);
    let mut high = 0u32;
    // SAFETY: buf is NUL-terminated; high is a valid out-param.
    let low = unsafe { GetCompressedFileSizeW(buf.as_ptr(), &mut high) };
    if low == u32::MAX {
        // SAFETY: plain error query.
        let err = unsafe { GetLastError() };
        if err != 0 {
            return None;
        }
    }
    Some(((high as u64) << 32) | low as u64)
}
