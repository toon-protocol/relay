//! `cargo xtask <task>` (#283). One task: `ui-assets`.

mod ui_assets;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask ui-assets [--out <dir>]\n\
    \n\
    ui-assets  build the app UIs' WebAssembly (needs the wasm32-unknown-unknown\n\
               target and a wasm-bindgen-cli matching the wasm-bindgen crate) into\n\
               content-hashed files and a manifest.json, default target/ui-assets";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("ui-assets") => {
            let out = match (args.next().as_deref(), args.next(), args.next()) {
                (None, _, _) => ui_assets::workspace_root().join("target/ui-assets"),
                (Some("--out"), Some(dir), None) => PathBuf::from(dir),
                _ => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            };
            match ui_assets::run(&out) {
                Ok(manifest) => {
                    println!("wrote {} assets to {}", manifest.len(), out.display());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("xtask ui-assets: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
