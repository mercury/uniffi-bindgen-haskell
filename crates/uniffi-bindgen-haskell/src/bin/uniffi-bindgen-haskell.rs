use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use clap::Parser;
use uniffi_bindgen_haskell::{generate, CabalOptions, GenerateOptions};

#[derive(Parser)]
#[command(
    name = "uniffi-bindgen-haskell",
    version,
    about = "Generate Haskell bindings for a UniFFI component"
)]
struct Arguments {
    #[arg(long, value_name = "PATH")]
    library: Utf8PathBuf,

    #[arg(long, value_name = "DIR")]
    out_dir: Utf8PathBuf,

    #[arg(
        long,
        value_name = "FILE",
        help = "Emit a Cabal package file relative to --out-dir"
    )]
    cabal_file: Option<Utf8PathBuf>,

    #[arg(
        long,
        default_value = "uniffi-generated-bindings",
        value_name = "NAME",
        help = "Package name used by --cabal-file"
    )]
    cabal_package_name: String,

    #[arg(
        long,
        value_name = "NAME",
        help = "Native linker library used by --cabal-file; inferred from --library by default"
    )]
    cabal_native_library: Option<String>,

    #[arg(
        long,
        value_name = "DIR",
        help = "Native library search directory written to the generated Cabal package"
    )]
    cabal_extra_lib_dir: Option<Utf8PathBuf>,
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let cabal = arguments
        .cabal_file
        .map(|file_name| -> Result<CabalOptions> {
            let native_library = arguments
                .cabal_native_library
                .clone()
                .map(Ok)
                .unwrap_or_else(|| infer_native_library(&arguments.library))?;
            Ok(CabalOptions {
                file_name,
                package_name: arguments.cabal_package_name.clone(),
                native_library,
                extra_lib_dir: arguments.cabal_extra_lib_dir.clone(),
            })
        })
        .transpose()?;

    generate(GenerateOptions {
        library: arguments.library,
        out_dir: arguments.out_dir,
        cabal,
    })
}

fn infer_native_library(library: &Utf8PathBuf) -> Result<String> {
    let stem = library
        .file_stem()
        .context("--library path has no file name")?;
    Ok(stem.strip_prefix("lib").unwrap_or(stem).to_string())
}
