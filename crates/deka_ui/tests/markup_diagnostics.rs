//! Reject the retired class attribute spelling at the authored attribute span.
#[test]
fn retired_class_spelling_is_a_compile_error() {
    trybuild::TestCases::new().compile_fail("../../tests/rust-ui/class-name.rs");
}
