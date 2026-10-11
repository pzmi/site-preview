//! The authorized bootstrap boundary: the one place outside the filesystem adapter that may
//! obtain ambient filesystem authority and build the filesystem adapter.

use std::fs;
use std::path::Path;

pub struct Site {
    pub entries: usize,
    pub reader: fn(&Path) -> std::io::Result<Vec<u8>>,
}

pub fn open_root(path: &Path) -> std::io::Result<Site> {
    let entries = fs::read_dir(path)?.count();
    Ok(Site {
        entries,
        reader: preview_adapter_fs::read,
    })
}
