use std::collections::HashMap;

use anyhow::{Context, Result};
use camino::Utf8Path;
use uniffi_bindgen::{BindgenLoader, BindgenPaths, GlobalConfig};

use crate::ir::{self, Bindings, FfiDefinition, FfiType};

pub(crate) fn load(library: &Utf8Path) -> Result<Bindings> {
    let loader = BindgenLoader::new(BindgenPaths::default(), GlobalConfig::default());
    let metadata = loader
        .load_metadata(library)
        .with_context(|| format!("failed to load UniFFI metadata from {library}"))?;
    let initial_root = loader.load_pipeline_initial_root(library, metadata)?;
    let mut pipeline = ir::general::pipeline("haskell");
    let mut root = pipeline.execute(initial_root)?;
    normalize_borrowed_byte_arguments(&mut root);

    let namespaces = root
        .namespaces
        .into_values()
        .map(|interface| {
            let module_segment = upper_camel(&interface.name);
            let public_module = format!("UniFFI.{module_segment}");
            let internal_module = format!("{public_module}.Internal");
            let c_stem = format!("uniffi_{}_haskell", sanitize_identifier(&interface.name));
            ir::Namespace {
                interface,
                module_segment,
                public_module,
                internal_module,
                c_stem,
            }
        })
        .collect();

    Ok(Bindings { namespaces })
}

fn normalize_borrowed_byte_arguments(root: &mut ir::general::Root) {
    for namespace in root.namespaces.values_mut() {
        let mut borrowed_arguments = HashMap::<String, Vec<usize>>::new();
        for function in &mut namespace.functions {
            let mut indices = Vec::new();
            for (index, argument) in function.callable.arguments.iter_mut().enumerate() {
                if argument.is_borrowed_bytes() {
                    argument.ty.ffi_type = FfiType::ForeignBytes;
                    indices.push(index);
                }
            }
            if !indices.is_empty() {
                borrowed_arguments.insert(function.callable.ffi_func.0.clone(), indices);
            }
        }

        let definitions = std::mem::take(&mut namespace.ffi_definitions);
        namespace.ffi_definitions = definitions
            .into_iter()
            .map(|mut definition| {
                if let FfiDefinition::RustFunction(function) = &mut definition {
                    if let Some(indices) = borrowed_arguments.get(&function.name.0) {
                        for index in indices {
                            function.arguments[*index].ty = FfiType::ForeignBytes;
                        }
                    }
                }
                definition
            })
            .collect();
    }
}

fn upper_camel(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

fn sanitize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
