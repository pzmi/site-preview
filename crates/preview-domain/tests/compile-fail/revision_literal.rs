// A `Revision` is not constructible from a bare number, so a `SiteState` cannot carry one
// that did not come from `Revision::ZERO` / `next()`.
use preview_domain::{InstanceId, Revision, SiteState};

fn main() {
    let instance = InstanceId::from_parts(1, 2);
    let _ = SiteState::new(instance, Revision(7));
}
