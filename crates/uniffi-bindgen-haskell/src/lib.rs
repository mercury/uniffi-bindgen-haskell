use anyhow::Result;
use camino::Utf8PathBuf;

mod ir;
mod renderer;
mod uniffi_adapter;

pub struct CabalOptions {
    pub file_name: Utf8PathBuf,
    pub header: String,
    pub native_library: String,
    pub extra_lib_dir: Option<Utf8PathBuf>,
}

pub struct GenerateOptions {
    pub library: Utf8PathBuf,
    pub out_dir: Utf8PathBuf,
    pub cabal: Option<CabalOptions>,
}

pub fn generate(options: GenerateOptions) -> Result<()> {
    let bindings = uniffi_adapter::load(&options.library)?;
    renderer::write(&bindings, &options.out_dir, options.cabal.as_ref())
}
