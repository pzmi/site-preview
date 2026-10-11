// `into_fd` consumes the file, so a borrowed (including mutably borrowed) `OpenedFile` cannot
// hand out its descriptor, and the file cannot be used after the descriptor left.
use preview_adapter_fs::OpenedFile;

fn steal(file: &mut OpenedFile) {
    let _fd = file.into_fd();
}

fn main() {}
