//! Read-only Windows disk usage inspector.
//!
//! The crate is split into a UI layer (`app`), a pure data model (`model`), a
//! background scanner (`scan`) and the Win32 layer (`platform`). Only
//! `platform` and `scan` touch FFI; nothing anywhere opens a path for writing.

pub mod app;
pub mod format;
pub mod model;
pub mod platform;
pub mod scan;
