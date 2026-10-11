//! Positive fixture: the CLI parses and delegates to composition, the only crate it names.

pub fn main_exit(root: &std::path::Path) -> u8 {
    preview_composition::run(root)
}
