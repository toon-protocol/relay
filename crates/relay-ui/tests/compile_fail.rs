//! The invariant the read-only handle holds: an app UI can query the store
//! and nothing else. Each file under `compile_fail/` is a use that must not
//! build, kept beside the compiler's reason.
//!
//! After a deliberate change or a toolchain bump, regenerate the reasons with
//! `TRYBUILD=overwrite cargo test -p relay-ui --test compile_fail`.

#[test]
fn the_forbidden_uses_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
