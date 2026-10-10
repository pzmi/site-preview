//! Positive fixture: composition wires adapters; only `bootstrap` opens the root.

mod bootstrap;

pub use bootstrap::open_root;

pub fn wire() -> fn(&std::path::Path) -> std::io::Result<Vec<u8>> {
    preview_adapter_fs::read
}
