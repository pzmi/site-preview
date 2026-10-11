// A `RootDir` is built only by `RootDir::open_root`: its descriptor field is private.
use preview_adapter_fs::RootDir;
use std::os::fd::OwnedFd;

fn root_from(fd: OwnedFd) -> RootDir {
    RootDir { fd }
}

fn main() {}
