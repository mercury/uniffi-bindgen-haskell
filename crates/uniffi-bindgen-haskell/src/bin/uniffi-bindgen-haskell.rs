use std::env;

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;
use uniffi_bindgen_haskell::{generate, GenerateOptions};

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    let mut library = None;
    let mut out_dir = None;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--library" => {
                library = Some(Utf8PathBuf::from(
                    arguments.next().context("--library requires a path")?,
                ));
            }
            "--out-dir" => {
                out_dir = Some(Utf8PathBuf::from(
                    arguments.next().context("--out-dir requires a path")?,
                ));
            }
            "--help" | "-h" => {
                println!("Usage: uniffi-bindgen-haskell --library PATH --out-dir DIR");
                return Ok(());
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    generate(GenerateOptions {
        library: library.context("missing --library PATH")?,
        out_dir: out_dir.context("missing --out-dir DIR")?,
    })
}
