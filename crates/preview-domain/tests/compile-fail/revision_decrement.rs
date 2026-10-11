// A `Revision` only moves forward through `next()`: its counter is private.
use preview_domain::Revision;

fn main() {
    let mut revision = Revision::ZERO;
    revision.0 -= 1;
}
