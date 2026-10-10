//! Positive fixture: the filesystem adapter holds ambient filesystem authority.

use std::fs;
use std::path::Path;

#[cfg(target_os = "linux")]
pub use rustix::fs::Mode;

pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    fs::read(path)
}
