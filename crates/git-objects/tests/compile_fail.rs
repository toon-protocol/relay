//! The invariant the type system holds: a `RawObject` exists only once its
//! hash was checked. Each file under `compile_fail/` is a construction that
//! must not build, kept beside the compiler's reason for refusing it.
//!
//! After a deliberate change or a toolchain bump, regenerate the reasons with
//! `TRYBUILD=overwrite cargo test -p git-objects --test compile_fail`.

#[test]
fn an_object_cannot_be_built_without_verifying_it() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
