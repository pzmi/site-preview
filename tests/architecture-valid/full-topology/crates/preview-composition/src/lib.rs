//! Positive fixture: composition wires adapters; only `bootstrap` opens the root and names
//! the filesystem adapter, and the rest of composition receives the capability it returns.

mod bootstrap;

pub fn run(root: &std::path::Path) -> u8 {
    match bootstrap::open_root(root) {
        Ok(site) => serve(site),
        Err(_) => 1,
    }
}

fn serve(site: bootstrap::Site) -> u8 {
    u8::from(site.entries == 0)
}
