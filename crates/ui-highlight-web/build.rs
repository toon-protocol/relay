//! Compiles `syntaxes/` into the syntax set the module embeds. Done here,
//! not at run time, so the browser parses nothing: it loads a ready set.

use std::path::PathBuf;

use syntect::dumps::dump_to_file;
use syntect::parsing::SyntaxSetBuilder;

fn main() {
    println!("cargo:rerun-if-changed=syntaxes");
    let mut builder = SyntaxSetBuilder::new();
    builder
        .add_from_folder("syntaxes", true)
        .expect("the vendored syntaxes are valid");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    dump_to_file(&builder.build(), out.join("syntaxes.packdump"))
        .expect("the syntax set can be written");
}
