//! Port traits: what the application needs from the outside world.
//!
//! Each trait's documentation is its contract. The real adapters and the testkit fakes
//! implement the same contract, and the shared contract harnesses run it against both
//! (foundation.md section 5.1); a fake is never looser than production.
//!
//! | Port | Dispatch | Why |
//! |:--|:--|:--|
//! | [`SiteContent`] | `&dyn SiteContent` | queries take it as a trait object; synchronous, so object safe |
//! | [`ChangeSource`] | generic, static | asynchronous; one concrete source per process, known at compile time |
//! | [`Clock`] | either | synchronous and object safe |
//!
//! ADR 0038 records these shapes and the alternatives that were rejected.

mod change_source;
mod clock;
mod site_content;

pub use change_source::{ChangeBatch, ChangeSource, SourceFailure};
pub use clock::Clock;
pub use site_content::{ContentFile, ContentRefusal, ForbiddenReason, Opened, SiteContent};
