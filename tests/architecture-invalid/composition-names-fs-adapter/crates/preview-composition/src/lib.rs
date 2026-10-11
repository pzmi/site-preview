//! Fixture: composition reaches the filesystem adapter outside the bootstrap boundary.

mod bootstrap;

pub fn reread(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    preview_adapter_fs::read(path)
}
