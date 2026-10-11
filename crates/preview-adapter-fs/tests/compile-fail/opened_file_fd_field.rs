// The descriptor of an `OpenedFile` is reachable only through `into_fd`: the field is private.
use preview_adapter_fs::OpenedFile;

fn steal(file: &mut OpenedFile) {
    let _ = &mut file.fd;
}

fn main() {}
