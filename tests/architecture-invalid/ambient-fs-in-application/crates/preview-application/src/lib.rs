//! Fixture: the application opens a file itself instead of asking a port.

use std::fs::File;

pub fn open_index() -> std::io::Result<File> {
    File::open("index.html")
}
