//! Positive fixture: the application uses domain types and tokio's sync primitives.

use preview_domain::Revision;
use tokio::sync::watch;

pub fn publish(sender: &watch::Sender<u64>, revision: &Revision) {
    let _ = sender.send(revision.0);
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn should_read_fixture_when_test_builds_a_site() {
        // given
        let dir = std::env::temp_dir();

        // when
        let listed = fs::read_dir(dir).is_ok();

        // then
        assert!(listed);
    }
}
