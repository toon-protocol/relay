//! The version this relay reports, on `GET /health` and in the Relay
//! Information Document.
//!
//! The crate stays at `0.1.0` and is never published, so its own version says
//! nothing about what is running. The release handle does: the image build
//! passes `TOON_RELEASE_HANDLE` (the connector's date-handle scheme,
//! `YYYY.MM.DD.N`, e.g. `2026.10.01.1`) and it is compiled in. A build made
//! without one, such as `cargo build` on a workstation, reports the crate
//! version instead.

/// The version reported to clients.
pub(crate) const VERSION: &str = match option_env!("TOON_RELEASE_HANDLE") {
    Some(handle) if !handle.is_empty() => handle,
    _ => env!("CARGO_PKG_VERSION"),
};

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn a_build_without_a_release_handle_reports_the_crate_version() {
        if option_env!("TOON_RELEASE_HANDLE").is_none() {
            assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        }
        assert!(!VERSION.is_empty());
    }
}
