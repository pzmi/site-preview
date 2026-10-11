// A `SitePath` exists only as the output of `decode_request_path`: its fields are private.
use preview_domain::SitePath;

fn main() {
    let _ = SitePath {
        components: Vec::new(),
        trailing_slash: false,
    };
}
