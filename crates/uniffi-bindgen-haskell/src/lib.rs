use anyhow::Result;
use camino::Utf8PathBuf;

mod ir;
mod renderer;
mod uniffi_adapter;

pub struct GenerateOptions {
    pub library: Utf8PathBuf,
    pub out_dir: Utf8PathBuf,
}

pub fn generate(options: GenerateOptions) -> Result<()> {
    let bindings = uniffi_adapter::load(&options.library)?;
    renderer::write(&bindings, &options.out_dir)
}
