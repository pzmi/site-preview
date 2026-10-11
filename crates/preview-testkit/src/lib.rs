//! Reusable handwritten test fakes; never a production dependency.
//!
//! Each fake implements a port from `preview-application` with the same contract as the real
//! adapter, never a looser one (foundation.md section 5.1): the shared contract harnesses run
//! against the fakes and the adapters alike.
//!
//! | Fake | Port | Notes |
//! |:--|:--|:--|
//! | [`InMemorySite`] | `SiteContent` | a tree of directories, files and link markers |
//! | [`FakeClock`] | `Clock` | advances only when told to |
//! | [`ScriptedChangeSource`] | `ChangeSource` | fed through a [`ScriptedHandle`] |
//! | [`FixedInstanceIds`] | none | deterministic [`InstanceId`](preview_domain::InstanceId)s |
//!
//! The shared contract harnesses live in [`contracts`]: [`contracts::site_content_contract`]
//! runs the `SiteContent` contract against any [`contracts::SiteContentFactory`], and
//! [`InMemorySite`] is one.

pub mod change_source;
pub mod clock;
pub mod contracts;
pub mod ids;
pub mod poll;
pub mod site;

pub use change_source::{ScriptedChangeSource, ScriptedHandle};
pub use clock::FakeClock;
pub use ids::FixedInstanceIds;
pub use site::InMemorySite;
