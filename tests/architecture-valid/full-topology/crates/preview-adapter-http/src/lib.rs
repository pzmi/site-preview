//! Positive fixture: the HTTP adapter uses application and domain types only.

use preview_application::publish;

pub fn handler() -> fn(&tokio_free::Sender, &preview_domain::Revision) {
    publish
}

mod tokio_free {
    pub type Sender = ();
}
