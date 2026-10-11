//! Compile-fail API fixtures (foundation.md section 3.1, ARCH-08): the domain types whose
//! invariant is "only constructible through a checked path" must keep refusing the shortcuts.
//! Each fixture under `tests/compile-fail/` pins its diagnostic in a `.stderr` file, so a
//! fixture that starts failing for another reason is a failing test, not a silent pass.

#[test]
fn should_reject_unchecked_construction_when_domain_fields_are_private() {
    // given
    let cases = trybuild::TestCases::new();

    // when
    cases.compile_fail("tests/compile-fail/*.rs");

    // then: trybuild verifies each fixture fails with its pinned `.stderr` when `cases` drops
}
