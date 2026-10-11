// A `Component` is validated by `Component::new`: its field is private.
use preview_domain::Component;

fn main() {
    let _ = Component(String::from(".."));
}
