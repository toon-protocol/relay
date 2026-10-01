//! The invariants the type system holds (#185, seam 2). Each file under
//! `compile_fail/` is a construction that must not build, kept beside the
//! compiler's reason for refusing it, so a change that makes one of them
//! build, or fail for a different reason, turns this test red.
//!
//! After a deliberate change or a toolchain bump, regenerate the reasons with
//! `TRYBUILD=overwrite cargo test -p relay --test compile_fail`.

#[test]
fn the_forbidden_constructions_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
