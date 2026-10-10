//! The authorized bootstrap boundary: the one place outside the filesystem adapter that may
//! obtain ambient filesystem authority.

use std::fs::{self, ReadDir};
use std::path::Path;

pub fn open_root(path: &Path) -> std::io::Result<ReadDir> {
    fs::read_dir(path)
}
