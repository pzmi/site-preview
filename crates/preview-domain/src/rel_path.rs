//! A root-relative path made of validated [`Component`]s, as the watcher reports it.
//!
//! The adapter turns an OS path into a `RelPath` (non-UTF-8 names are its refusal to make);
//! the domain never sees an absolute path or touches the filesystem. The empty path is the
//! site root.

use crate::path::Component;

/// Root-relative components, outermost first. The empty path is the site root.
///
/// Ordering is component-wise, so a directory sorts immediately before its descendants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RelPath(Vec<Component>);

impl RelPath {
    /// The site root.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// Builds a path from validated components.
    #[must_use]
    pub fn new(components: Vec<Component>) -> Self {
        Self(components)
    }

    /// The components, outermost first.
    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.0
    }

    /// Whether this is the site root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The path of the child called `name`.
    #[must_use]
    pub fn join(&self, name: Component) -> Self {
        let mut components = self.0.clone();
        components.push(name);
        Self(components)
    }

    /// The containing directory, or `None` for the root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let (_, rest) = self.0.split_last()?;
        Some(Self(rest.to_vec()))
    }

    /// Whether `self` is `other` or lies beneath it (the root contains everything).
    #[must_use]
    pub fn starts_with(&self, other: &Self) -> bool {
        self.0.starts_with(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(parts: &[&str]) -> RelPath {
        RelPath::new(
            parts
                .iter()
                .map(|p| Component::new(p).expect("valid test component"))
                .collect(),
        )
    }

    #[test]
    fn should_return_parent_when_path_is_not_root() {
        // given
        let path = rel(&["a", "b"]);

        // when
        let parent = path.parent();
        let root_parent = RelPath::root().parent();

        // then
        assert_eq!(parent, Some(rel(&["a"])));
        assert_eq!(root_parent, None);
    }

    #[test]
    fn should_match_prefix_by_whole_component_when_checking_descendants() {
        // given
        let dir = rel(&["css"]);

        // when
        let inside = rel(&["css", "site.css"]).starts_with(&dir);
        let sibling = rel(&["css-extra", "x"]).starts_with(&dir);
        let everything = dir.starts_with(&RelPath::root());

        // then
        assert!(inside);
        assert!(!sibling);
        assert!(everything);
    }

    #[test]
    fn should_extend_path_when_joining_a_component() {
        // given
        let dir = rel(&["a"]);

        // when
        let joined = dir.join(Component::new("b").expect("valid"));

        // then
        assert_eq!(joined, rel(&["a", "b"]));
    }
}
