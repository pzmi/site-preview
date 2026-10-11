//! Fixture: a query publishes the next revision while it answers a read.

use crate::reload_state::ReloadState;

pub struct Read;

impl Read {
    pub fn latest(&self, state: &ReloadState) -> u64 {
        state.publish_next()
    }
}
