//! Compile-fail API fixtures (foundation.md section 3.1, ARCH-08): the filesystem capability
//! types must keep refusing construction from a raw descriptor and descriptor extraction
//! through a borrow. Each fixture under `tests/compile-fail/` pins its diagnostic in a
//! `.stderr` file.

#[test]
fn should_reject_capability_shortcuts_when_adapter_fields_are_private() {
    // given
    let cases = trybuild::TestCases::new();

    // when
    cases.compile_fail("tests/compile-fail/*.rs");

    // then: trybuild verifies each fixture fails with its pinned `.stderr` when `cases` drops
}
