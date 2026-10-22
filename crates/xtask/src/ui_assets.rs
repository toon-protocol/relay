//! `cargo xtask ui-assets`: build each app UI's WebAssembly crate, run
//! `wasm-bindgen` over it and write a folder of content-hashed files plus a
//! `manifest.json` from logical names to hashed ones (the folder
//! `TOON_UI_ASSETS_DIR` points the relay at).
//!
//! Per crate `<name>` (`ui-nip34-web`), the manifest has:
//! - `<name>.wasm`: the module;
//! - `<name>.glue.js`: wasm-bindgen's loader, rewritten to fetch the hashed
//!   module;
//! - `<name>.js`: a one-line module that loads the glue and starts it, which
//!   is what a page's `<script type="module">` names (a page may carry no
//!   inline script).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// The WebAssembly crates, by package name.
const WEB_CRATES: &[&str] = &["ui-nip34-web"];

const TARGET: &str = "wasm32-unknown-unknown";

/// Hex digits of the digest in a file name.
const HASH_LEN: usize = 16;

pub(crate) fn run(out: &Path) -> Result<BTreeMap<String, String>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target_dir = PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or("target".into()));
    let bindgen_dir = target_dir.join("ui-assets-bindgen");

    let mut build = Command::new(&cargo);
    build.args(["build", "--release", "--locked", "--target", TARGET]);
    for package in WEB_CRATES {
        build.args(["-p", package]);
    }
    status(&mut build, "cargo build")?;

    if out.exists() {
        std::fs::remove_dir_all(out)
            .map_err(|error| format!("clearing {}: {error}", out.display()))?;
    }
    std::fs::create_dir_all(out).map_err(|error| format!("creating {}: {error}", out.display()))?;

    let mut manifest = BTreeMap::new();
    for package in WEB_CRATES {
        let module = package.replace('-', "_");
        let input = target_dir
            .join(TARGET)
            .join("release")
            .join(format!("{module}.wasm"));
        let mut bindgen = Command::new("wasm-bindgen");
        bindgen
            .args([
                "--target",
                "web",
                "--no-typescript",
                "--out-name",
                &module,
                "--out-dir",
            ])
            .arg(&bindgen_dir)
            .arg(&input);
        status(&mut bindgen, "wasm-bindgen")?;

        let wasm = read(&bindgen_dir.join(format!("{module}_bg.wasm")))?;
        let glue = String::from_utf8(read(&bindgen_dir.join(format!("{module}.js")))?)
            .map_err(|error| format!("{module}.js is not text: {error}"))?;

        let wasm_name = write_hashed(out, &module, "wasm", &wasm)?;
        let glue = point_glue_at(&glue, &format!("{module}_bg.wasm"), &wasm_name)?;
        let glue_name = write_hashed(out, &format!("{module}.glue"), "js", glue.as_bytes())?;
        let boot_name = write_hashed(out, &module, "js", boot(&glue_name).as_bytes())?;

        manifest.insert(format!("{package}.wasm"), wasm_name);
        manifest.insert(format!("{package}.glue.js"), glue_name);
        manifest.insert(format!("{package}.js"), boot_name);
    }
    write_manifest(out, &manifest)?;
    Ok(manifest)
}

fn status(command: &mut Command, what: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("running {what}: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{what} failed: {status}"))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("reading {}: {error}", path.display()))
}

/// The file name `stem.<hash>.ext`, the hash being the SHA-256 of `bytes`.
pub(crate) fn hashed_name(stem: &str, extension: &str, bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hash = String::new();
    for byte in digest.iter().take(HASH_LEN / 2) {
        let _ = write!(hash, "{byte:02x}");
    }
    format!("{stem}.{hash}.{extension}")
}

fn write_hashed(out: &Path, stem: &str, extension: &str, bytes: &[u8]) -> Result<String, String> {
    let name = hashed_name(stem, extension, bytes);
    let path = out.join(&name);
    std::fs::write(&path, bytes).map_err(|error| format!("writing {}: {error}", path.display()))?;
    Ok(name)
}

/// wasm-bindgen's loader names its module by a literal relative URL; the
/// module was renamed by its hash, so the literal must follow. The loader
/// not naming it is an error: it would fetch a file that is not there.
pub(crate) fn point_glue_at(glue: &str, original: &str, hashed: &str) -> Result<String, String> {
    if !glue.contains(original) {
        return Err(format!(
            "the loader does not name {original}; wasm-bindgen changed its output"
        ));
    }
    Ok(glue.replace(original, hashed))
}

/// The module a page loads: starts the glue, which fetches the module beside it.
pub(crate) fn boot(glue_name: &str) -> String {
    format!("import init from './{glue_name}';\nawait init();\n")
}

fn write_manifest(out: &Path, manifest: &BTreeMap<String, String>) -> Result<(), String> {
    let json = serde_json::to_string_pretty(manifest).map_err(|error| error.to_string())?;
    let path = out.join("manifest.json");
    std::fs::write(&path, json + "\n")
        .map_err(|error| format!("writing {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_carries_a_hash_of_the_bytes() {
        let a = hashed_name("app", "wasm", b"one");
        assert_eq!(a, hashed_name("app", "wasm", b"one"));
        assert_ne!(a, hashed_name("app", "wasm", b"two"));
        assert!(a.starts_with("app.") && a.ends_with(".wasm"));
        assert_eq!(a.len(), "app..wasm".len() + HASH_LEN);
    }

    #[test]
    fn the_loader_is_pointed_at_the_hashed_module() {
        let glue = "new URL('app_bg.wasm', import.meta.url)";
        assert_eq!(
            point_glue_at(glue, "app_bg.wasm", "app.abc.wasm").unwrap(),
            "new URL('app.abc.wasm', import.meta.url)"
        );
        assert!(point_glue_at("nothing", "app_bg.wasm", "x").is_err());
    }
}
