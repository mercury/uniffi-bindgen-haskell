use std::fmt::Write as _;
use std::fs;

use anyhow::{bail, Context, Result};
use camino::Utf8Path;
use serde::Serialize;

use crate::ir::{
    general, Bindings, FfiDefinition, FfiFunction, FfiFunctionType, FfiStruct, FfiType, FieldsKind,
    Namespace as NamespacePlan, Type, TypeDefinition, UniFfiNamespace,
};
use crate::CabalOptions;

type Namespace = UniFfiNamespace;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    uniffi_version: &'static str,
    runtime_abi_version: u32,
    haskell_modules: Vec<String>,
    public_haskell_modules: Vec<String>,
    internal_haskell_modules: Vec<String>,
    haskell_source_dirs: Vec<String>,
    c_sources: Vec<String>,
    headers: Vec<String>,
    required_native_libraries: Vec<String>,
}

pub(crate) fn write(
    bindings: &Bindings,
    out_dir: &Utf8Path,
    cabal: Option<&CabalOptions>,
) -> Result<()> {
    fs::create_dir_all(out_dir)?;

    let mut manifest = Manifest {
        schema_version: 2,
        uniffi_version: "0.32.0",
        runtime_abi_version: 1,
        haskell_modules: Vec::new(),
        public_haskell_modules: Vec::new(),
        internal_haskell_modules: Vec::new(),
        haskell_source_dirs: vec!["haskell".to_string()],
        c_sources: Vec::new(),
        headers: Vec::new(),
        required_native_libraries: Vec::new(),
    };

    for namespace in &bindings.namespaces {
        render_namespace(namespace, out_dir, &mut manifest)?;
    }

    let manifest_path = out_dir.join("manifest.json");
    fs::write(
        manifest_path,
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    if let Some(cabal) = cabal {
        write_cabal_file(out_dir, &manifest, cabal)?;
    }
    Ok(())
}

fn write_cabal_file(out_dir: &Utf8Path, manifest: &Manifest, options: &CabalOptions) -> Result<()> {
    let file_name = options.file_name.as_str();
    if options.file_name.is_absolute() || file_name.contains('/') || file_name.contains('\\') {
        bail!("--cabal-file must be a file name relative to --out-dir");
    }
    if options.header.trim().is_empty() {
        bail!("--cabal-header must not be empty");
    }

    let mut output = options.header.clone();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    if !output.ends_with("\n\n") {
        output.push('\n');
    }
    writeln!(output, "library")?;
    writeln!(output, "  exposed-modules:")?;
    for module in &manifest.public_haskell_modules {
        writeln!(output, "      {module}")?;
    }
    writeln!(output, "  other-modules:")?;
    for module in &manifest.internal_haskell_modules {
        writeln!(output, "      {module}")?;
    }
    writeln!(output, "  hs-source-dirs: haskell")?;
    writeln!(output, "  build-depends:")?;
    writeln!(output, "      base >=4.15 && <5")?;
    writeln!(output, "    , bytestring >=0.10 && <0.13")?;
    writeln!(output, "    , containers >=0.6 && <0.8")?;
    writeln!(output, "    , text >=1.2 && <2.2")?;
    writeln!(output, "    , uniffi-runtime")?;
    writeln!(output, "  include-dirs: cbits")?;
    writeln!(output, "  c-sources:")?;
    for source in &manifest.c_sources {
        writeln!(output, "      {source}")?;
    }
    writeln!(output, "  cc-options: -std=c11")?;
    if let Some(extra_lib_dir) = &options.extra_lib_dir {
        writeln!(output, "  extra-lib-dirs: {extra_lib_dir}")?;
    }
    writeln!(output, "  extra-libraries:")?;
    writeln!(output, "      {}", options.native_library)?;
    writeln!(output, "      c")?;
    writeln!(output, "      m")?;
    writeln!(output, "  default-language: GHC2021")?;
    writeln!(output, "  ghc-options: -Wall")?;

    let path = out_dir.join(&options.file_name);
    create_parent(&path)?;
    fs::write(path, output)?;
    Ok(())
}

fn render_namespace(
    namespace: &NamespacePlan,
    out_dir: &Utf8Path,
    manifest: &mut Manifest,
) -> Result<()> {
    let interface = &namespace.interface;
    let module_segment = &namespace.module_segment;
    let module_name = &namespace.public_module;
    let module_path = out_dir
        .join("haskell")
        .join("UniFFI")
        .join(format!("{module_segment}.hs"));
    let internal_module_name = &namespace.internal_module;
    let internal_module_path = out_dir
        .join("haskell")
        .join("UniFFI")
        .join(module_segment)
        .join("Internal.hs");
    let c_stem = &namespace.c_stem;
    let header_relative = format!("cbits/{c_stem}.h");
    let source_relative = format!("cbits/{c_stem}.c");
    let header_path = out_dir.join(&header_relative);
    let source_path = out_dir.join(&source_relative);

    create_parent(&module_path)?;
    create_parent(&internal_module_path)?;
    create_parent(&header_path)?;
    fs::write(&module_path, render_haskell_public(interface, module_name)?)?;
    fs::write(
        &internal_module_path,
        render_haskell(interface, module_name)?,
    )?;
    fs::write(&header_path, render_c_header(interface, c_stem)?)?;
    fs::write(
        &source_path,
        render_c_source(interface, &format!("{c_stem}.h"))?,
    )?;

    manifest.haskell_modules.push(module_name.clone());
    manifest.haskell_modules.push(internal_module_name.clone());
    manifest.public_haskell_modules.push(module_name.clone());
    manifest
        .internal_haskell_modules
        .push(internal_module_name.clone());
    manifest.c_sources.push(source_relative);
    manifest.headers.push(header_relative);
    manifest
        .required_native_libraries
        .push(interface.crate_name.clone());
    Ok(())
}

fn create_parent(path: &Utf8Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn render_c_header(namespace: &Namespace, c_stem: &str) -> Result<String> {
    let guard = format!("{}_H", c_stem.to_ascii_uppercase());
    let mut out = String::new();
    writeln!(out, "#ifndef {guard}")?;
    writeln!(out, "#define {guard}")?;
    writeln!(out)?;
    writeln!(out, "#include <stddef.h>")?;
    writeln!(out, "#include <stdint.h>")?;
    writeln!(out)?;
    writeln!(out, "typedef struct RustBuffer {{")?;
    writeln!(out, "    uint64_t capacity;")?;
    writeln!(out, "    uint64_t len;")?;
    writeln!(out, "    uint8_t *data;")?;
    writeln!(out, "}} RustBuffer;")?;
    writeln!(out)?;
    writeln!(out, "typedef struct ForeignBytes {{")?;
    writeln!(out, "    int32_t len;")?;
    writeln!(out, "    const uint8_t *data;")?;
    writeln!(out, "}} ForeignBytes;")?;
    writeln!(out)?;
    writeln!(out, "typedef struct RustCallStatus {{")?;
    writeln!(out, "    int8_t code;")?;
    writeln!(out, "    RustBuffer error_buf;")?;
    writeln!(out, "}} RustCallStatus;")?;
    writeln!(out)?;
    writeln!(
        out,
        "_Static_assert(offsetof(RustBuffer, capacity) == 0, \"RustBuffer capacity offset\");"
    )?;
    writeln!(
        out,
        "_Static_assert(offsetof(RustBuffer, len) == sizeof(uint64_t), \"RustBuffer len offset\");"
    )?;
    writeln!(
        out,
        "_Static_assert(offsetof(RustCallStatus, error_buf) % _Alignof(RustBuffer) == 0, \"RustCallStatus alignment\");"
    )?;
    writeln!(out)?;

    for definition in &namespace.ffi_definitions {
        match definition {
            FfiDefinition::FunctionType(function_type) => {
                render_c_function_type(&mut out, function_type)?;
            }
            FfiDefinition::Struct(struct_) => render_c_struct(&mut out, struct_)?,
            FfiDefinition::RustFunction(function) => {
                render_c_raw_function(&mut out, function)?;
                render_c_adapter_declaration(&mut out, function)?;
            }
        }
    }
    for definition in &namespace.type_definitions {
        if let TypeDefinition::CallbackInterface(callback) = definition {
            writeln!(
                out,
                "void {}(void);",
                callback_installer_symbol(namespace, callback)
            )?;
        }
    }

    writeln!(out)?;
    writeln!(out, "#endif")?;
    Ok(out)
}

fn render_c_function_type(out: &mut String, function: &FfiFunctionType) -> Result<()> {
    let return_type = c_return_type(function.return_type.ty.as_ref())?;
    let arguments = c_raw_arguments(
        &function.arguments,
        function.has_rust_call_status_arg,
        false,
    )?;
    writeln!(
        out,
        "typedef {return_type} (*{})({arguments});",
        function.name.0
    )?;
    Ok(())
}

fn render_c_struct(out: &mut String, struct_: &FfiStruct) -> Result<()> {
    writeln!(out, "typedef struct {} {{", struct_.name.0)?;
    for field in &struct_.fields {
        writeln!(out, "    {} {};", c_type(&field.ty)?, field.name)?;
    }
    writeln!(out, "}} {};", struct_.name.0)?;
    Ok(())
}

fn render_c_raw_function(out: &mut String, function: &FfiFunction) -> Result<()> {
    let return_type = c_return_type(function.return_type.ty.as_ref())?;
    let arguments = c_raw_arguments(
        &function.arguments,
        function.has_rust_call_status_arg,
        false,
    )?;
    writeln!(out, "{return_type} {}({arguments});", function.name.0)?;
    Ok(())
}

fn render_c_adapter_declaration(out: &mut String, function: &FfiFunction) -> Result<()> {
    let return_type = adapter_return_type(function.return_type.ty.as_ref())?;
    let arguments = c_adapter_arguments(function)?;
    writeln!(
        out,
        "{return_type} {}({arguments});",
        adapter_symbol(&function.name.0)
    )?;
    Ok(())
}

fn render_c_source(namespace: &Namespace, header_name: &str) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "#include \"{header_name}\"")?;
    writeln!(out)?;
    for definition in &namespace.ffi_definitions {
        if let FfiDefinition::RustFunction(function) = definition {
            render_c_adapter_definition(&mut out, function)?;
        }
    }
    for definition in &namespace.type_definitions {
        if let TypeDefinition::CallbackInterface(callback) = definition {
            render_c_callback_interface(&mut out, namespace, callback)?;
        }
    }
    Ok(out)
}

fn callback_symbol_prefix(namespace: &Namespace, callback: &general::CallbackInterface) -> String {
    format!(
        "hs_callback_{}_{}",
        sanitize_identifier(&namespace.name),
        sanitize_identifier(&callback.name)
    )
}

fn callback_installer_symbol(
    namespace: &Namespace,
    callback: &general::CallbackInterface,
) -> String {
    format!("{}_install", callback_symbol_prefix(namespace, callback))
}

fn render_c_callback_interface(
    out: &mut String,
    namespace: &Namespace,
    callback: &general::CallbackInterface,
) -> Result<()> {
    let struct_name = match &callback.vtable.struct_type {
        FfiType::Struct(name) => &name.0,
        other => bail!("callback vtable had unexpected type {other:?}"),
    };
    let struct_definition = namespace
        .ffi_definitions
        .iter()
        .find_map(|definition| match definition {
            FfiDefinition::Struct(struct_) if struct_.name.0 == *struct_name => Some(struct_),
            _ => None,
        })
        .with_context(|| format!("missing callback vtable struct {struct_name}"))?;
    let prefix = callback_symbol_prefix(namespace, callback);
    let mut trampoline_names = Vec::new();

    for field in &struct_definition.fields {
        let function_name = match &field.ty {
            FfiType::Function(name) => &name.0,
            other => bail!("callback vtable field {} had type {other:?}", field.name),
        };
        let function_type = namespace
            .ffi_definitions
            .iter()
            .find_map(|definition| match definition {
                FfiDefinition::FunctionType(function) if function.name.0 == *function_name => {
                    Some(function)
                }
                _ => None,
            })
            .with_context(|| format!("missing callback function type {function_name}"))?;
        let implementation_symbol = format!("{prefix}_{}", sanitize_identifier(&field.name));
        let trampoline_symbol = format!("{implementation_symbol}_trampoline");
        trampoline_names.push(trampoline_symbol.clone());
        let return_type = c_return_type(function_type.return_type.ty.as_ref())?;
        let haskell_arguments = c_raw_arguments(
            &function_type.arguments,
            function_type.has_rust_call_status_arg,
            true,
        )?;
        let raw_arguments = c_raw_arguments(
            &function_type.arguments,
            function_type.has_rust_call_status_arg,
            false,
        )?;
        writeln!(
            out,
            "extern {return_type} {implementation_symbol}({haskell_arguments});"
        )?;
        writeln!(
            out,
            "static {return_type} {trampoline_symbol}({raw_arguments}) {{"
        )?;
        let mut call_arguments = function_type
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                if is_by_value_struct(&argument.ty) {
                    format!("&arg_{index}")
                } else {
                    format!("arg_{index}")
                }
            })
            .collect::<Vec<_>>();
        if function_type.has_rust_call_status_arg {
            call_arguments.push("out_status".to_string());
        }
        let call = format!("{implementation_symbol}({})", call_arguments.join(", "));
        if function_type.return_type.ty.is_some() {
            writeln!(out, "    return {call};")?;
        } else {
            writeln!(out, "    {call};")?;
        }
        writeln!(out, "}}")?;
        writeln!(out)?;
    }

    let vtable_symbol = format!("{prefix}_vtable");
    writeln!(out, "static const {struct_name} {vtable_symbol} = {{")?;
    for trampoline in &trampoline_names {
        writeln!(out, "    {trampoline},")?;
    }
    writeln!(out, "}};")?;
    writeln!(out)?;
    writeln!(
        out,
        "void {}(void) {{",
        callback_installer_symbol(namespace, callback)
    )?;
    writeln!(out, "    {}(&{vtable_symbol});", callback.vtable.init_fn.0)?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    Ok(())
}

fn render_c_adapter_definition(out: &mut String, function: &FfiFunction) -> Result<()> {
    let return_type = adapter_return_type(function.return_type.ty.as_ref())?;
    let arguments = c_adapter_arguments(function)?;
    writeln!(
        out,
        "{return_type} {}({arguments}) {{",
        adapter_symbol(&function.name.0)
    )?;

    let mut call_arguments = Vec::new();
    for (index, argument) in function.arguments.iter().enumerate() {
        let name = format!("arg_{index}");
        if is_by_value_struct(&argument.ty) {
            call_arguments.push(format!("*{name}"));
        } else {
            call_arguments.push(name);
        }
    }
    if function.has_rust_call_status_arg {
        call_arguments.push("out_status".to_string());
    }
    let call = format!("{}({})", function.name.0, call_arguments.join(", "));

    match function.return_type.ty.as_ref() {
        None => writeln!(out, "    {call};")?,
        Some(ty) if is_by_value_struct(ty) => writeln!(out, "    *out_return = {call};")?,
        Some(_) => writeln!(out, "    return {call};")?,
    }
    writeln!(out, "}}")?;
    writeln!(out)?;
    Ok(())
}

fn c_raw_arguments(
    arguments: &[general::FfiArgument],
    has_status: bool,
    adapt_structs: bool,
) -> Result<String> {
    let mut rendered = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let ty = if adapt_structs && is_by_value_struct(&argument.ty) {
            format!("const {} *", c_type(&argument.ty)?)
        } else {
            c_type(&argument.ty)?
        };
        rendered.push(format!("{ty} arg_{index}"));
    }
    if has_status {
        rendered.push("RustCallStatus *out_status".to_string());
    }
    if rendered.is_empty() {
        Ok("void".to_string())
    } else {
        Ok(rendered.join(", "))
    }
}

fn c_adapter_arguments(function: &FfiFunction) -> Result<String> {
    let mut rendered = Vec::new();
    for (index, argument) in function.arguments.iter().enumerate() {
        let ty = if is_by_value_struct(&argument.ty) {
            format!("const {} *", c_type(&argument.ty)?)
        } else {
            c_type(&argument.ty)?
        };
        rendered.push(format!("{ty} arg_{index}"));
    }
    if let Some(ty) = function.return_type.ty.as_ref() {
        if is_by_value_struct(ty) {
            rendered.push(format!("{} *out_return", c_type(ty)?));
        }
    }
    if function.has_rust_call_status_arg {
        rendered.push("RustCallStatus *out_status".to_string());
    }
    if rendered.is_empty() {
        Ok("void".to_string())
    } else {
        Ok(rendered.join(", "))
    }
}

fn adapter_return_type(return_type: Option<&FfiType>) -> Result<String> {
    match return_type {
        Some(ty) if is_by_value_struct(ty) => Ok("void".to_string()),
        other => c_return_type(other),
    }
}

fn c_return_type(return_type: Option<&FfiType>) -> Result<String> {
    match return_type {
        Some(ty) => c_type(ty),
        None => Ok("void".to_string()),
    }
}

fn c_type(ty: &FfiType) -> Result<String> {
    Ok(match ty {
        FfiType::UInt8 => "uint8_t".to_string(),
        FfiType::Int8 => "int8_t".to_string(),
        FfiType::UInt16 => "uint16_t".to_string(),
        FfiType::Int16 => "int16_t".to_string(),
        FfiType::UInt32 => "uint32_t".to_string(),
        FfiType::Int32 => "int32_t".to_string(),
        FfiType::UInt64 | FfiType::Handle(_) => "uint64_t".to_string(),
        FfiType::Int64 => "int64_t".to_string(),
        FfiType::Float32 => "float".to_string(),
        FfiType::Float64 => "double".to_string(),
        FfiType::RustBuffer(_) => "RustBuffer".to_string(),
        FfiType::ForeignBytes => "ForeignBytes".to_string(),
        FfiType::Function(name) => name.0.clone(),
        FfiType::Struct(name) => name.0.clone(),
        FfiType::RustCallStatus => "RustCallStatus".to_string(),
        FfiType::Reference(inner) => format!("const {} *", c_type(inner)?),
        FfiType::MutReference(inner) => format!("{} *", c_type(inner)?),
        FfiType::VoidPointer => "void *".to_string(),
    })
}

fn is_by_value_struct(ty: &FfiType) -> bool {
    matches!(ty, FfiType::RustBuffer(_) | FfiType::ForeignBytes)
}

fn render_haskell_public(namespace: &Namespace, module_name: &str) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "module {module_name}")?;
    writeln!(out, "  ( initialize")?;
    for definition in &namespace.type_definitions {
        match definition {
            TypeDefinition::Record(record) => {
                let type_name = upper_camel(&record.name);
                writeln!(out, "  , {type_name} (..)")?;
                if record.fields.iter().all(|field| field.default.is_some()) {
                    writeln!(out, "  , default{type_name}")?;
                }
            }
            TypeDefinition::Enum(enum_) => {
                writeln!(out, "  , {} (..)", upper_camel(&enum_.name))?;
            }
            TypeDefinition::Custom(custom) => {
                writeln!(out, "  , {} (..)", upper_camel(&custom.name))?;
            }
            TypeDefinition::CallbackInterface(callback) => {
                writeln!(out, "  , {} (..)", upper_camel(&callback.name))?;
            }
            TypeDefinition::Interface(interface) => {
                let type_name = upper_camel(&interface.name);
                writeln!(out, "  , {type_name}")?;
                writeln!(out, "  , close{type_name}")?;
                for constructor in &interface.constructors {
                    writeln!(
                        out,
                        "  , {}",
                        haskell_constructor_name(&type_name, constructor)
                    )?;
                }
                for method in &interface.methods {
                    writeln!(
                        out,
                        "  , {}",
                        haskell_method_name(&type_name, &method.callable.name)
                    )?;
                }
            }
            _ => {}
        }
    }
    for function in &namespace.functions {
        let function_name = haskell_value_name(&function.callable.name);
        writeln!(out, "  , {function_name}")?;
        if function
            .callable
            .arguments
            .iter()
            .any(|argument| argument.default.is_some())
        {
            writeln!(out, "  , {function_name}UsingDefaults")?;
        }
    }
    writeln!(out, "  ) where")?;
    writeln!(out)?;
    writeln!(out, "import {module_name}.Internal")?;
    Ok(out)
}

fn render_haskell(namespace: &Namespace, module_name: &str) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "{{-# LANGUAGE DuplicateRecordFields #-}}")?;
    writeln!(out, "{{-# LANGUAGE ForeignFunctionInterface #-}}")?;
    writeln!(out, "{{-# LANGUAGE OverloadedRecordDot #-}}")?;
    writeln!(out, "{{-# LANGUAGE TypeApplications #-}}")?;
    writeln!(
        out,
        "{{-# OPTIONS_GHC -Wno-unused-imports -Wno-unused-top-binds #-}}"
    )?;
    writeln!(out)?;
    writeln!(out, "module {module_name}.Internal where")?;
    writeln!(out)?;
    writeln!(
        out,
        "import Control.Exception (SomeException, displayException, throwIO, try)"
    )?;
    writeln!(out, "import Control.Monad (unless)")?;
    writeln!(out, "import Data.ByteString (ByteString)")?;
    writeln!(out, "import Data.Int (Int8, Int16, Int32, Int64)")?;
    writeln!(out, "import Data.Map.Strict (Map)")?;
    writeln!(out, "import qualified Data.Map.Strict as Map")?;
    writeln!(out, "import Data.Set (Set)")?;
    writeln!(out, "import qualified Data.Set as Set")?;
    writeln!(out, "import Data.Text (Text)")?;
    writeln!(out, "import qualified Data.Text as Text")?;
    writeln!(out, "import Data.Word (Word8, Word16, Word32, Word64)")?;
    writeln!(out, "import Foreign.Marshal.Alloc (alloca)")?;
    writeln!(out, "import Foreign.Marshal.Utils (with)")?;
    writeln!(out, "import Foreign.Ptr (Ptr)")?;
    writeln!(out, "import Foreign.Storable (peek, poke)")?;
    writeln!(out, "import Prelude hiding (readList)")?;
    writeln!(out, "import System.IO.Unsafe (unsafePerformIO)")?;
    writeln!(out, "import UniFFI.Runtime")?;
    for definition in &namespace.type_definitions {
        if let TypeDefinition::External(external) = definition {
            let external_module = upper_camel(&external.namespace);
            let type_name = upper_camel(&external.name);
            writeln!(
                out,
                "import UniFFI.{external_module}.Internal ({type_name} (..), decode{type_name}, encode{type_name})"
            )?;
        }
    }
    writeln!(out)?;

    render_haskell_runtime_imports(&mut out, namespace)?;
    for function in &namespace.functions {
        render_haskell_ffi_import(&mut out, function)?;
    }
    for definition in &namespace.type_definitions {
        match definition {
            TypeDefinition::Interface(interface) => {
                render_haskell_interface_ffi_imports(&mut out, interface)?;
            }
            TypeDefinition::CallbackInterface(callback) => {
                render_haskell_callback_ffi(&mut out, namespace, callback)?;
            }
            _ => {}
        }
    }
    writeln!(out)?;
    render_initialize(&mut out, namespace)?;

    for definition in &namespace.type_definitions {
        match definition {
            TypeDefinition::Record(record) => {
                writeln!(out)?;
                render_haskell_record(&mut out, record)?;
            }
            TypeDefinition::Enum(enum_) => {
                writeln!(out)?;
                render_haskell_enum(&mut out, enum_)?;
            }
            TypeDefinition::Custom(custom) => {
                writeln!(out)?;
                render_haskell_custom_type(&mut out, custom)?;
            }
            TypeDefinition::CallbackInterface(callback) => {
                writeln!(out)?;
                render_haskell_callback_interface(&mut out, namespace, callback)?;
            }
            TypeDefinition::Interface(interface) => {
                writeln!(out)?;
                render_haskell_interface(&mut out, interface)?;
            }
            _ => {}
        }
    }

    for function in &namespace.functions {
        writeln!(out)?;
        render_haskell_function(&mut out, function)?;
        if function
            .callable
            .arguments
            .iter()
            .any(|argument| argument.default.is_some())
        {
            writeln!(out)?;
            render_haskell_default_function(&mut out, function)?;
        }
    }
    Ok(out)
}

fn lower_camel_type_name(type_name: &str) -> String {
    let mut characters = type_name.chars();
    match characters.next() {
        Some(first) => first.to_lowercase().chain(characters).collect(),
        None => String::new(),
    }
}

fn callback_field_name(type_name: &str, method_name: &str) -> String {
    format!(
        "{}{}",
        lower_camel_type_name(type_name),
        upper_camel(method_name)
    )
}

fn render_haskell_callback_ffi(
    out: &mut String,
    namespace: &Namespace,
    callback: &general::CallbackInterface,
) -> Result<()> {
    let type_name = upper_camel(&callback.name);
    let prefix = callback_symbol_prefix(namespace, callback);
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_install{type_name} :: IO ()",
        callback_installer_symbol(namespace, callback)
    )?;
    writeln!(
        out,
        "foreign export ccall \"{prefix}_uniffi_free\" callback{type_name}Free :: Word64 -> IO ()"
    )?;
    writeln!(
        out,
        "foreign export ccall \"{prefix}_uniffi_clone\" callback{type_name}Clone :: Word64 -> IO Word64"
    )?;
    for method in &callback.methods {
        let handler_name = format!("callback{type_name}{}", upper_camel(&method.callable.name));
        let symbol = format!("{prefix}_{}", sanitize_identifier(&method.callable.name));
        let mut signature = vec!["Word64".to_string()];
        signature.extend(
            method
                .callable
                .arguments
                .iter()
                .map(|argument| haskell_ffi_argument_type(&argument.ty.ffi_type))
                .collect::<Result<Vec<_>>>()?,
        );
        let out_type = match method.callable.return_type.ty.as_ref() {
            Some(return_type) => {
                format!("Ptr {}", haskell_ffi_pointee_type(&return_type.ffi_type)?)
            }
            None => "Ptr ()".to_string(),
        };
        signature.push(out_type);
        signature.push("Ptr RustCallStatus".to_string());
        signature.push("IO ()".to_string());
        writeln!(
            out,
            "foreign export ccall \"{symbol}\" {handler_name} :: {}",
            signature.join(" -> ")
        )?;
    }
    Ok(())
}

fn render_haskell_callback_interface(
    out: &mut String,
    _namespace: &Namespace,
    callback: &general::CallbackInterface,
) -> Result<()> {
    let type_name = upper_camel(&callback.name);
    writeln!(out, "data {type_name} = {type_name}")?;
    for (index, method) in callback.methods.iter().enumerate() {
        let prefix = if index == 0 { "  {" } else { "  ," };
        let mut signature = method
            .callable
            .arguments
            .iter()
            .map(haskell_argument_api_type)
            .collect::<Result<Vec<_>>>()?;
        let return_type = method
            .callable
            .return_type
            .ty
            .as_ref()
            .map(|return_type| haskell_api_type(&return_type.ty))
            .transpose()?
            .unwrap_or_else(|| "()".to_string());
        let io_result = match method.callable.throws_type.ty.as_ref() {
            Some(error_type) => format!(
                "IO (Either {} {return_type})",
                haskell_api_type(&error_type.ty)?
            ),
            None => format!("IO ({return_type})"),
        };
        signature.push(io_result);
        writeln!(
            out,
            "{prefix} {} :: {}",
            callback_field_name(&type_name, &method.callable.name),
            signature.join(" -> ")
        )?;
    }
    writeln!(out, "  }}")?;
    writeln!(out)?;
    writeln!(out, "callback{type_name}Free :: Word64 -> IO ()")?;
    writeln!(out, "callback{type_name}Free = freeCallbackHandle")?;
    writeln!(out)?;
    writeln!(out, "callback{type_name}Clone :: Word64 -> IO Word64")?;
    writeln!(out, "callback{type_name}Clone = cloneCallbackHandle")?;

    for method in &callback.methods {
        writeln!(out)?;
        render_haskell_callback_method(out, &type_name, method)?;
    }
    Ok(())
}

fn render_haskell_callback_method(
    out: &mut String,
    type_name: &str,
    method: &general::Method,
) -> Result<()> {
    let callable = &method.callable;
    if callable.async_data.is_some() {
        bail!(
            "async callback method {} is not supported yet",
            callable.orig_name
        );
    }
    let handler_name = format!("callback{type_name}{}", upper_camel(&callable.name));
    let mut handler_signature = vec!["Word64".to_string()];
    handler_signature.extend(
        callable
            .arguments
            .iter()
            .map(|argument| haskell_ffi_argument_type(&argument.ty.ffi_type))
            .collect::<Result<Vec<_>>>()?,
    );
    handler_signature.push(match callable.return_type.ty.as_ref() {
        Some(return_type) => format!("Ptr {}", haskell_ffi_pointee_type(&return_type.ffi_type)?),
        None => "Ptr ()".to_string(),
    });
    handler_signature.push("Ptr RustCallStatus".to_string());
    handler_signature.push("IO ()".to_string());
    writeln!(out, "{handler_name} :: {}", handler_signature.join(" -> "))?;
    let ffi_argument_names: Vec<String> = (0..callable.arguments.len())
        .map(|index| format!("ffiArg{index}"))
        .collect();
    let mut parameters = vec!["handle".to_string()];
    parameters.extend(ffi_argument_names.iter().cloned());
    parameters.push("outReturn".to_string());
    parameters.push("statusPointer".to_string());
    writeln!(out, "{handler_name} {} = do", parameters.join(" "))?;
    writeln!(out, "  outcome <- try @SomeException $ do")?;
    writeln!(
        out,
        "    callback <- dereferenceCallbackHandle handle :: IO {type_name}"
    )?;
    let mut lifted_names = Vec::new();
    for ((argument, ffi_name), index) in callable
        .arguments
        .iter()
        .zip(&ffi_argument_names)
        .zip(0usize..)
    {
        let lifted_name = format!("argument{index}");
        render_callback_argument_lift(
            out,
            "    ",
            &argument.ty.ty,
            &argument.ty.ffi_type,
            ffi_name,
            &lifted_name,
        )?;
        lifted_names.push(lifted_name);
    }
    let field_name = callback_field_name(type_name, &callable.name);
    let call = if lifted_names.is_empty() {
        format!("callback.{field_name}")
    } else {
        format!("callback.{field_name} {}", lifted_names.join(" "))
    };
    writeln!(out, "    {call}")?;
    writeln!(out, "  case outcome of")?;
    writeln!(
        out,
        "    Left exception -> setCallbackUnexpectedError c_rustBufferFromBytes c_rustBufferFree (displayException exception) statusPointer"
    )?;
    if let Some(error_type) = callable.throws_type.ty.as_ref() {
        writeln!(out, "    Right (Left callbackError) ->")?;
        writeln!(
            out,
            "      setCallbackError c_rustBufferFromBytes c_rustBufferFree (runEncoder ({})) statusPointer",
            encoder_expression(&error_type.ty, "callbackError")?
        )?;
        writeln!(out, "    Right (Right callbackResult) -> do")?;
        render_callback_result_lower(
            out,
            "      ",
            callable
                .return_type
                .ty
                .as_ref()
                .map(|return_type| &return_type.ty),
            "callbackResult",
            "outReturn",
        )?;
    } else {
        writeln!(out, "    Right callbackResult -> do")?;
        render_callback_result_lower(
            out,
            "      ",
            callable
                .return_type
                .ty
                .as_ref()
                .map(|return_type| &return_type.ty),
            "callbackResult",
            "outReturn",
        )?;
    }
    Ok(())
}

fn render_callback_argument_lift(
    out: &mut String,
    indent: &str,
    api_type: &Type,
    ffi_type: &FfiType,
    ffi_name: &str,
    lifted_name: &str,
) -> Result<()> {
    match ffi_type {
        FfiType::RustBuffer(_) => {
            writeln!(out, "{indent}{lifted_name}Buffer <- peek {ffi_name}")?;
            writeln!(
                out,
                "{indent}{lifted_name}Bytes <- consumeRustBuffer c_rustBufferFree {lifted_name}Buffer"
            )?;
            match api_type {
                Type::String => {
                    writeln!(out, "{indent}{lifted_name} <-")?;
                    writeln!(out, "{indent}  case decodeUtf8 {lifted_name}Bytes of")?;
                    writeln!(
                        out,
                        "{indent}    Left err -> throwIO (UniFFIException (Text.pack (show err)))"
                    )?;
                    writeln!(out, "{indent}    Right value -> pure value")?;
                }
                other => writeln!(
                    out,
                    "{indent}{lifted_name} <- runDecoder ({}) {lifted_name}Bytes",
                    decoder_expression(other)?
                )?,
            }
        }
        _ => match api_type {
            Type::Boolean => {
                writeln!(out, "{indent}{lifted_name} <-")?;
                writeln!(out, "{indent}  case {ffi_name} of")?;
                writeln!(out, "{indent}    0 -> pure False")?;
                writeln!(out, "{indent}    1 -> pure True")?;
                writeln!(
                    out,
                    "{indent}    other -> throwIO (UniFFIException (Text.pack (show other)))"
                )?;
            }
            _ => writeln!(out, "{indent}let {lifted_name} = {ffi_name}")?,
        },
    }
    Ok(())
}

fn render_callback_result_lower(
    out: &mut String,
    indent: &str,
    return_type: Option<&Type>,
    value: &str,
    out_pointer: &str,
) -> Result<()> {
    match return_type {
        None => writeln!(out, "{indent}pure ()")?,
        Some(Type::Boolean) => {
            writeln!(out, "{indent}poke {out_pointer} (if {value} then 1 else 0)")?
        }
        Some(Type::String) => {
            writeln!(
                out,
                "{indent}buffer <- lowerRustBuffer c_rustBufferFromBytes c_rustBufferFree (encodeUtf8 {value})"
            )?;
            writeln!(out, "{indent}poke {out_pointer} buffer")?;
        }
        Some(
            ty @ (Type::Bytes
            | Type::Optional { .. }
            | Type::Sequence { .. }
            | Type::Map { .. }
            | Type::Set { .. }
            | Type::Timestamp
            | Type::Duration
            | Type::Box { .. }
            | Type::Record { .. }
            | Type::Enum { .. }
            | Type::Custom { .. }),
        ) => {
            writeln!(
                out,
                "{indent}buffer <- lowerRustBuffer c_rustBufferFromBytes c_rustBufferFree (runEncoder ({}))",
                encoder_expression(ty, value)?
            )?;
            writeln!(out, "{indent}poke {out_pointer} buffer")?;
        }
        Some(
            Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64
            | Type::Float32
            | Type::Float64,
        ) => writeln!(out, "{indent}poke {out_pointer} {value}")?,
        Some(other) => bail!("callback return lowering for {other:?} is not supported yet"),
    }
    Ok(())
}

fn render_haskell_custom_type(out: &mut String, custom: &general::CustomType) -> Result<()> {
    let type_name = upper_camel(&custom.name);
    writeln!(
        out,
        "newtype {type_name} = {type_name} {}",
        haskell_api_type(&custom.builtin.ty)?
    )?;
    writeln!(out, "  deriving (Eq, Ord, Show)")?;
    writeln!(out)?;
    writeln!(
        out,
        "unwrap{type_name} :: {type_name} -> {}",
        haskell_api_type(&custom.builtin.ty)?
    )?;
    writeln!(out, "unwrap{type_name} ({type_name} value) = value")?;
    writeln!(out)?;
    writeln!(out, "encode{type_name} :: {type_name} -> Encoder")?;
    writeln!(
        out,
        "encode{type_name} ({type_name} value) = {}",
        encoder_expression(&custom.builtin.ty, "value")?
    )?;
    writeln!(out)?;
    writeln!(out, "decode{type_name} :: Decoder {type_name}")?;
    writeln!(
        out,
        "decode{type_name} = {type_name} <$> {}",
        decoder_expression(&custom.builtin.ty)?
    )?;
    Ok(())
}

fn render_haskell_interface_ffi_imports(
    out: &mut String,
    interface: &general::Interface,
) -> Result<()> {
    let type_name = upper_camel(&interface.name);
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_clone{type_name} :: Word64 -> Ptr RustCallStatus -> IO Word64",
        adapter_symbol(&interface.ffi_func_clone.0)
    )?;
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_free{type_name} :: Word64 -> Ptr RustCallStatus -> IO ()",
        adapter_symbol(&interface.ffi_func_free.0)
    )?;
    for constructor in &interface.constructors {
        render_haskell_object_callable_ffi_import(
            out,
            &constructor.callable,
            &haskell_constructor_name(&type_name, constructor),
            false,
        )?;
    }
    for method in &interface.methods {
        render_haskell_object_callable_ffi_import(
            out,
            &method.callable,
            &haskell_method_name(&type_name, &method.callable.name),
            true,
        )?;
    }
    Ok(())
}

fn render_haskell_object_callable_ffi_import(
    out: &mut String,
    callable: &general::Callable,
    binding_name: &str,
    has_receiver: bool,
) -> Result<()> {
    if let Some(async_data) = callable.async_data.as_ref() {
        return render_haskell_async_ffi_import_named(
            out,
            callable,
            async_data,
            binding_name,
            has_receiver,
        );
    }
    let mut parts = Vec::new();
    if has_receiver {
        parts.push("Word64".to_string());
    }
    for argument in &callable.arguments {
        parts.push(haskell_ffi_argument_type(&argument.ty.ffi_type)?);
    }
    if let Some(return_type) = callable.return_type.ty.as_ref() {
        if is_by_value_struct(&return_type.ffi_type) {
            parts.push(format!(
                "Ptr {}",
                haskell_ffi_struct_type(&return_type.ffi_type)?
            ));
        }
    }
    parts.push("Ptr RustCallStatus".to_string());
    let return_type = match callable.return_type.ty.as_ref() {
        Some(return_type) if !is_by_value_struct(&return_type.ffi_type) => {
            haskell_ffi_scalar_type(&return_type.ffi_type)?
        }
        _ => "()".to_string(),
    };
    parts.push(format!("IO {return_type}"));
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{binding_name} :: {}",
        adapter_symbol(&callable.ffi_func.0),
        parts.join(" -> ")
    )?;
    Ok(())
}

fn render_haskell_interface(out: &mut String, interface: &general::Interface) -> Result<()> {
    let type_name = upper_camel(&interface.name);
    writeln!(out, "newtype {type_name} = {type_name} RustObject")?;
    writeln!(out)?;
    writeln!(out, "release{type_name} :: Word64 -> IO ()")?;
    writeln!(out, "release{type_name} handle = do")?;
    writeln!(out, "  (_, status) <-")?;
    writeln!(out, "    withRustCallStatus $ \\statusPtr ->")?;
    writeln!(out, "      c_free{type_name} handle statusPtr")?;
    writeln!(out, "  checkRustCallStatus c_rustBufferFree status")?;
    writeln!(out)?;
    writeln!(out, "close{type_name} :: {type_name} -> IO ()")?;
    writeln!(
        out,
        "close{type_name} ({type_name} object) = finalizeRustObject object"
    )?;
    writeln!(out)?;
    writeln!(out, "clone{type_name}Handle :: {type_name} -> IO Word64")?;
    writeln!(out, "clone{type_name}Handle ({type_name} object) =")?;
    writeln!(out, "  withRustObject object $ \\handle -> do")?;
    writeln!(out, "    (clonedHandle, status) <-")?;
    writeln!(out, "      withRustCallStatus $ \\statusPtr ->")?;
    writeln!(out, "        c_clone{type_name} handle statusPtr")?;
    writeln!(out, "    checkRustCallStatus c_rustBufferFree status")?;
    writeln!(out, "    pure clonedHandle")?;

    for constructor in &interface.constructors {
        writeln!(out)?;
        render_haskell_constructor(out, &type_name, constructor)?;
    }
    for method in &interface.methods {
        writeln!(out)?;
        render_haskell_method(out, &type_name, method)?;
    }
    Ok(())
}

fn render_haskell_constructor(
    out: &mut String,
    type_name: &str,
    constructor: &general::Constructor,
) -> Result<()> {
    let callable = &constructor.callable;
    let function_name = haskell_constructor_name(type_name, constructor);
    let argument_names: Vec<String> = callable
        .arguments
        .iter()
        .map(|argument| haskell_value_name(&argument.name))
        .collect();
    let mut signature = callable
        .arguments
        .iter()
        .map(haskell_argument_api_type)
        .collect::<Result<Vec<_>>>()?;
    signature.push(format!("IO {type_name}"));
    writeln!(out, "{function_name} :: {}", signature.join(" -> "))?;
    if argument_names.is_empty() {
        writeln!(out, "{function_name} = do")?;
    } else {
        writeln!(out, "{function_name} {} = do", argument_names.join(" "))?;
    }
    writeln!(out, "  initialize")?;
    let lowered = callable
        .arguments
        .iter()
        .zip(&argument_names)
        .map(|(argument, name)| {
            lower_argument_expression(&argument.ty.ty, &argument.ty.ffi_type, name)
        })
        .collect::<Result<Vec<_>>>()?;
    let args = append_call_arguments(&lowered, &["statusPtr"]);
    writeln!(out, "  (handle, status) <-")?;
    writeln!(out, "    withRustCallStatus $ \\statusPtr ->")?;
    writeln!(out, "      c_{function_name} {args}")?;
    writeln!(out, "  checkRustCallStatus c_rustBufferFree status")?;
    writeln!(
        out,
        "  {type_name} <$> newRustObject handle release{type_name}"
    )?;
    Ok(())
}

fn render_haskell_method(
    out: &mut String,
    type_name: &str,
    method: &general::Method,
) -> Result<()> {
    let callable = &method.callable;
    let function_name = haskell_method_name(type_name, &callable.name);
    let argument_names: Vec<String> = callable
        .arguments
        .iter()
        .map(|argument| haskell_value_name(&argument.name))
        .collect();
    let mut signature = vec![type_name.to_string()];
    signature.extend(
        callable
            .arguments
            .iter()
            .map(haskell_argument_api_type)
            .collect::<Result<Vec<_>>>()?,
    );
    let result_type = callable
        .return_type
        .ty
        .as_ref()
        .map(|return_type| haskell_api_type(&return_type.ty))
        .transpose()?
        .unwrap_or_else(|| "()".to_string());
    let io_result = match callable.throws_type.ty.as_ref() {
        Some(error_type) => format!(
            "IO (Either {} {result_type})",
            haskell_api_type(&error_type.ty)?
        ),
        None => format!("IO ({result_type})"),
    };
    signature.push(io_result);
    writeln!(out, "{function_name} :: {}", signature.join(" -> "))?;
    let all_arguments = std::iter::once("object".to_string())
        .chain(argument_names.iter().cloned())
        .collect::<Vec<_>>();
    writeln!(out, "{function_name} {} = do", all_arguments.join(" "))?;
    writeln!(out, "  initialize")?;
    for (argument, name) in callable.arguments.iter().zip(&argument_names) {
        if matches!(argument.ty.ty, Type::CallbackInterface { .. }) {
            writeln!(out, "  callback_{name}_handle <- newCallbackHandle {name}")?;
        }
        if let Type::Interface {
            name: interface_name,
            ..
        } = &argument.ty.ty
        {
            writeln!(
                out,
                "  object_{name}_handle <- clone{}Handle {name}",
                upper_camel(interface_name)
            )?;
        }
        if matches!(argument.ty.ffi_type, FfiType::RustBuffer(_)) {
            let encoded = match &argument.ty.ty {
                Type::String => format!("encodeUtf8 {name}"),
                other => {
                    writeln!(
                        out,
                        "  let serialized_{name} = runEncoder ({})",
                        encoder_expression(other, name)?
                    )?;
                    format!("serialized_{name}")
                }
            };
            writeln!(
                out,
                "  lowered_{name} <- lowerRustBuffer c_rustBufferFromBytes c_rustBufferFree ({encoded})"
            )?;
        }
    }
    writeln!(out, "  clonedHandle <- clone{type_name}Handle object")?;
    let mut indent = "  ".to_string();
    for (argument, name) in callable.arguments.iter().zip(&argument_names) {
        match &argument.ty.ffi_type {
            FfiType::RustBuffer(_) => {
                writeln!(
                    out,
                    "{indent}with lowered_{name} $ \\lowered_{name}_ptr -> do"
                )?;
                indent.push_str("  ");
            }
            FfiType::ForeignBytes => {
                let with_function = if is_mutable_borrowed_bytes(argument) {
                    "withMutableForeignBytes"
                } else {
                    "withForeignBytes"
                };
                writeln!(
                    out,
                    "{indent}{with_function} {name} $ \\borrowed_{name}_ptr -> do"
                )?;
                indent.push_str("  ");
            }
            _ => {}
        }
    }
    let mut lowered = vec!["clonedHandle".to_string()];
    lowered.extend(
        callable
            .arguments
            .iter()
            .zip(&argument_names)
            .map(|(argument, name)| {
                lower_argument_expression(&argument.ty.ty, &argument.ty.ffi_type, name)
            })
            .collect::<Result<Vec<_>>>()?,
    );
    if callable.async_data.is_some() {
        render_haskell_async_body(out, callable, &function_name, &lowered, &indent)?;
        return Ok(());
    }
    match callable.return_type.ty.as_ref() {
        Some(return_type) if is_by_value_struct(&return_type.ffi_type) => {
            writeln!(out, "{indent}alloca $ \\returnPtr -> do")?;
            let inner = format!("{indent}  ");
            writeln!(out, "{inner}poke returnPtr emptyRustBuffer")?;
            let args = append_call_arguments(&lowered, &["returnPtr", "statusPtr"]);
            writeln!(out, "{inner}(_, status) <-")?;
            writeln!(out, "{inner}  withRustCallStatus $ \\statusPtr ->")?;
            writeln!(out, "{inner}    c_{function_name} {args}")?;
            let success_indent = if let Some(error_type) = callable.throws_type.ty.as_ref() {
                writeln!(
                    out,
                    "{inner}callError <- checkRustCallStatusWithError c_rustBufferFree ({}) status",
                    decoder_expression(&error_type.ty)?
                )?;
                writeln!(out, "{inner}case callError of")?;
                writeln!(out, "{inner}  Just err -> pure (Left err)")?;
                writeln!(out, "{inner}  Nothing -> do")?;
                format!("{inner}    ")
            } else {
                writeln!(out, "{inner}checkRustCallStatus c_rustBufferFree status")?;
                inner.clone()
            };
            writeln!(out, "{success_indent}returnBuffer <- peek returnPtr")?;
            writeln!(
                out,
                "{success_indent}returnBytes <- consumeRustBuffer c_rustBufferFree returnBuffer"
            )?;
            render_lift_buffer(
                out,
                &success_indent,
                &return_type.ty,
                "returnBytes",
                callable.throws_type.ty.is_some(),
            )?;
        }
        return_type => {
            let args = append_call_arguments(&lowered, &["statusPtr"]);
            let return_binding = if return_type.is_some() {
                "returnValue"
            } else {
                "_"
            };
            writeln!(out, "{indent}({return_binding}, status) <-")?;
            writeln!(out, "{indent}  withRustCallStatus $ \\statusPtr ->")?;
            writeln!(out, "{indent}    c_{function_name} {args}")?;
            let success_indent = if let Some(error_type) = callable.throws_type.ty.as_ref() {
                writeln!(
                    out,
                    "{indent}callError <- checkRustCallStatusWithError c_rustBufferFree ({}) status",
                    decoder_expression(&error_type.ty)?
                )?;
                writeln!(out, "{indent}case callError of")?;
                writeln!(out, "{indent}  Just err -> pure (Left err)")?;
                writeln!(out, "{indent}  Nothing -> do")?;
                format!("{indent}    ")
            } else {
                writeln!(out, "{indent}checkRustCallStatus c_rustBufferFree status")?;
                indent.clone()
            };
            render_lift_scalar(
                out,
                &success_indent,
                return_type.map(|return_type| &return_type.ty),
                "returnValue",
                callable.throws_type.ty.is_some(),
            )?;
        }
    }
    Ok(())
}

fn haskell_constructor_name(type_name: &str, constructor: &general::Constructor) -> String {
    match constructor.callable.kind {
        general::CallableKind::Constructor { primary: true, .. } => format!("new{type_name}"),
        _ => format!(
            "{}{}",
            haskell_value_name(&constructor.callable.name),
            type_name
        ),
    }
}

fn haskell_method_name(type_name: &str, method_name: &str) -> String {
    format!(
        "{}{}",
        lower_camel_type_name(type_name),
        upper_camel(method_name)
    )
}

fn render_haskell_record(out: &mut String, record: &general::Record) -> Result<()> {
    let type_name = upper_camel(&record.name);
    match record.fields_kind {
        FieldsKind::Unit => {
            writeln!(out, "data {type_name} = {type_name}")?;
        }
        FieldsKind::Named => {
            writeln!(out, "data {type_name} = {type_name}")?;
            for (index, field) in record.fields.iter().enumerate() {
                let prefix = if index == 0 { "  {" } else { "  ," };
                writeln!(
                    out,
                    "{prefix} {} :: {}",
                    haskell_value_name(&field.name),
                    haskell_api_type(&field.ty.ty)?
                )?;
            }
            writeln!(out, "  }}")?;
        }
        FieldsKind::Unnamed => bail!("unnamed record {} is not supported", record.orig_name),
    }
    writeln!(out, "  deriving (Eq, Show)")?;
    if record.fields.iter().all(|field| field.default.is_some()) {
        writeln!(out)?;
        writeln!(out, "default{type_name} :: {type_name}")?;
        let defaults = record
            .fields
            .iter()
            .map(|field| render_default_value(field.default.as_ref().expect("checked above")))
            .collect::<Result<Vec<_>>>()?;
        if defaults.is_empty() {
            writeln!(out, "default{type_name} = {type_name}")?;
        } else {
            writeln!(
                out,
                "default{type_name} = {type_name} {}",
                defaults.join(" ")
            )?;
        }
    }
    writeln!(out)?;
    writeln!(out, "encode{type_name} :: {type_name} -> Encoder")?;
    if record.fields.is_empty() {
        writeln!(out, "encode{type_name} {type_name} = mempty")?;
    } else {
        writeln!(out, "encode{type_name} value =")?;
        render_encoder_chain(
            out,
            "  ",
            record
                .fields
                .iter()
                .map(|field| {
                    encoder_expression(
                        &field.ty.ty,
                        &format!("value.{}", haskell_value_name(&field.name)),
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        )?;
    }
    writeln!(out)?;
    writeln!(out, "decode{type_name} :: Decoder {type_name}")?;
    if record.fields.is_empty() {
        writeln!(out, "decode{type_name} = pure {type_name}")?;
    } else {
        let decoders = record
            .fields
            .iter()
            .map(|field| decoder_expression(&field.ty.ty))
            .collect::<Result<Vec<_>>>()?;
        write!(out, "decode{type_name} = {type_name} <$> {}", decoders[0])?;
        for decoder in &decoders[1..] {
            write!(out, " <*> {decoder}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

fn render_haskell_enum(out: &mut String, enum_: &general::Enum) -> Result<()> {
    let type_name = upper_camel(&enum_.name);
    for (index, variant) in enum_.variants.iter().enumerate() {
        let prefix = if index == 0 {
            format!("data {type_name} =")
        } else {
            "  |".to_string()
        };
        let constructor = format!("{}{}", type_name, upper_camel(&variant.name));
        let field_types = variant
            .fields
            .iter()
            .map(|field| haskell_api_type(&field.ty.ty))
            .collect::<Result<Vec<_>>>()?;
        if field_types.is_empty() {
            writeln!(out, "{prefix} {constructor}")?;
        } else {
            writeln!(out, "{prefix} {constructor} {}", field_types.join(" "))?;
        }
    }
    if enum_.variants.is_empty() {
        writeln!(out, "data {type_name}")?;
    }
    writeln!(out, "  deriving (Eq, Show)")?;
    writeln!(out)?;
    writeln!(out, "encode{type_name} :: {type_name} -> Encoder")?;
    writeln!(out, "encode{type_name} value =")?;
    writeln!(out, "  case value of")?;
    for (index, variant) in enum_.variants.iter().enumerate() {
        let constructor = format!("{}{}", type_name, upper_camel(&variant.name));
        let variables: Vec<String> = (0..variant.fields.len())
            .map(|field_index| format!("field{field_index}"))
            .collect();
        let pattern = if variables.is_empty() {
            constructor
        } else {
            format!("{constructor} {}", variables.join(" "))
        };
        let mut encoders = vec![format!("writeInt32 {}", index + 1)];
        encoders.extend(
            variant
                .fields
                .iter()
                .zip(&variables)
                .map(|(field, variable)| encoder_expression(&field.ty.ty, variable))
                .collect::<Result<Vec<_>>>()?,
        );
        write!(out, "    {pattern} -> ")?;
        writeln!(out, "{}", encoders.join(" <> "))?;
    }
    writeln!(out)?;
    writeln!(out, "decode{type_name} :: Decoder {type_name}")?;
    writeln!(out, "decode{type_name} = do")?;
    writeln!(out, "  tag <- readInt32")?;
    writeln!(out, "  case tag of")?;
    for (index, variant) in enum_.variants.iter().enumerate() {
        let constructor = format!("{}{}", type_name, upper_camel(&variant.name));
        let decoders = variant
            .fields
            .iter()
            .map(|field| decoder_expression(&field.ty.ty))
            .collect::<Result<Vec<_>>>()?;
        if decoders.is_empty() {
            writeln!(out, "    {} -> pure {constructor}", index + 1)?;
        } else {
            write!(
                out,
                "    {} -> {constructor} <$> {}",
                index + 1,
                decoders[0]
            )?;
            for decoder in &decoders[1..] {
                write!(out, " <*> {decoder}")?;
            }
            writeln!(out)?;
        }
    }
    writeln!(
        out,
        "    _ -> fail (\"invalid {type_name} variant: \" ++ show tag)"
    )?;
    Ok(())
}

fn render_encoder_chain(out: &mut String, indent: &str, encoders: Vec<String>) -> Result<()> {
    if encoders.is_empty() {
        writeln!(out, "{indent}mempty")?;
    } else {
        writeln!(out, "{indent}{}", encoders.join("\n    <> "))?;
    }
    Ok(())
}

fn encoder_expression(ty: &Type, value: &str) -> Result<String> {
    Ok(match ty {
        Type::UInt8 => format!("writeWord8 {value}"),
        Type::Int8 => format!("writeInt8 {value}"),
        Type::UInt16 => format!("writeWord16 {value}"),
        Type::Int16 => format!("writeInt16 {value}"),
        Type::UInt32 => format!("writeWord32 {value}"),
        Type::Int32 => format!("writeInt32 {value}"),
        Type::UInt64 => format!("writeWord64 {value}"),
        Type::Int64 => format!("writeInt64 {value}"),
        Type::Float32 => format!("writeFloat {value}"),
        Type::Float64 => format!("writeDouble {value}"),
        Type::Boolean => format!("writeBool {value}"),
        Type::String => format!("writeText {value}"),
        Type::Bytes => format!("writeBytes {value}"),
        Type::Optional { inner_type } => {
            format!(
                "writeMaybe (\\item -> {}) {value}",
                encoder_expression(inner_type, "item")?
            )
        }
        Type::Sequence { inner_type } => {
            format!(
                "writeList (\\item -> {}) {value}",
                encoder_expression(inner_type, "item")?
            )
        }
        Type::Map {
            key_type,
            value_type,
        } => format!(
            "writeMap (\\key -> {}) (\\item -> {}) {value}",
            encoder_expression(key_type, "key")?,
            encoder_expression(value_type, "item")?
        ),
        Type::Set { inner_type } => format!(
            "writeSet (\\item -> {}) {value}",
            encoder_expression(inner_type, "item")?
        ),
        Type::Timestamp => format!("writeTimestamp {value}"),
        Type::Duration => format!("writeDuration {value}"),
        Type::Box { inner_type } => encoder_expression(inner_type, value)?,
        Type::Record { name, .. } | Type::Enum { name, .. } | Type::Custom { name, .. } => {
            format!("encode{} {value}", upper_camel(name))
        }
        other => bail!("encoding {other:?} is not supported yet"),
    })
}

fn decoder_expression(ty: &Type) -> Result<String> {
    Ok(match ty {
        Type::UInt8 => "readWord8".to_string(),
        Type::Int8 => "readInt8".to_string(),
        Type::UInt16 => "readWord16".to_string(),
        Type::Int16 => "readInt16".to_string(),
        Type::UInt32 => "readWord32".to_string(),
        Type::Int32 => "readInt32".to_string(),
        Type::UInt64 => "readWord64".to_string(),
        Type::Int64 => "readInt64".to_string(),
        Type::Float32 => "readFloat".to_string(),
        Type::Float64 => "readDouble".to_string(),
        Type::Boolean => "readBool".to_string(),
        Type::String => "readText".to_string(),
        Type::Bytes => "readBytes".to_string(),
        Type::Optional { inner_type } => format!("readMaybe ({})", decoder_expression(inner_type)?),
        Type::Sequence { inner_type } => format!("readList ({})", decoder_expression(inner_type)?),
        Type::Map {
            key_type,
            value_type,
        } => format!(
            "readMap ({}) ({})",
            decoder_expression(key_type)?,
            decoder_expression(value_type)?
        ),
        Type::Set { inner_type } => format!("readSet ({})", decoder_expression(inner_type)?),
        Type::Timestamp => "readTimestamp".to_string(),
        Type::Duration => "readDuration".to_string(),
        Type::Box { inner_type } => decoder_expression(inner_type)?,
        Type::Record { name, .. } | Type::Enum { name, .. } | Type::Custom { name, .. } => {
            format!("decode{}", upper_camel(name))
        }
        other => bail!("decoding {other:?} is not supported yet"),
    })
}

fn render_haskell_runtime_imports(out: &mut String, namespace: &Namespace) -> Result<()> {
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_rustBufferFromBytes :: RustBufferFromBytes",
        adapter_symbol(&namespace.ffi_rustbuffer_from_bytes.0)
    )?;
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_rustBufferFree :: RustBufferFree",
        adapter_symbol(&namespace.ffi_rustbuffer_free.0)
    )?;
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_contractVersion :: IO Word32",
        adapter_symbol(&namespace.ffi_uniffi_contract_version.0)
    )?;
    for (index, checksum) in namespace.checksums.iter().enumerate() {
        writeln!(
            out,
            "foreign import ccall safe \"{}\" c_checksum_{index} :: IO Word16",
            adapter_symbol(&checksum.fn_name.0)
        )?;
    }
    Ok(())
}

fn render_haskell_async_ffi_import(
    out: &mut String,
    callable: &general::Callable,
    async_data: &general::AsyncData,
) -> Result<()> {
    render_haskell_async_ffi_import_named(
        out,
        callable,
        async_data,
        &haskell_value_name(&callable.name),
        false,
    )
}

fn render_haskell_async_ffi_import_named(
    out: &mut String,
    callable: &general::Callable,
    async_data: &general::AsyncData,
    function_name: &str,
    has_receiver: bool,
) -> Result<()> {
    let mut start_parts = Vec::new();
    if has_receiver {
        start_parts.push("Word64".to_string());
    }
    start_parts.extend(
        callable
            .arguments
            .iter()
            .map(|argument| haskell_ffi_argument_type(&argument.ty.ffi_type))
            .collect::<Result<Vec<_>>>()?,
    );
    start_parts.push("IO Word64".to_string());
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{function_name}_start :: {}",
        adapter_symbol(&callable.ffi_func.0),
        start_parts.join(" -> ")
    )?;
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{function_name}_poll :: RustFuturePoll",
        adapter_symbol(&async_data.ffi_rust_future_poll.0)
    )?;
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{function_name}_cancel :: RustFutureCancel",
        adapter_symbol(&async_data.ffi_rust_future_cancel.0)
    )?;
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{function_name}_free :: RustFutureFree",
        adapter_symbol(&async_data.ffi_rust_future_free.0)
    )?;
    let complete_type = match callable.return_type.ty.as_ref() {
        Some(return_type) if is_by_value_struct(&return_type.ffi_type) => {
            format!(
                "Word64 -> Ptr {} -> Ptr RustCallStatus -> IO ()",
                haskell_ffi_struct_type(&return_type.ffi_type)?
            )
        }
        Some(return_type) => format!(
            "Word64 -> Ptr RustCallStatus -> IO {}",
            haskell_ffi_scalar_type(&return_type.ffi_type)?
        ),
        None => "Word64 -> Ptr RustCallStatus -> IO ()".to_string(),
    };
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{function_name}_complete :: {complete_type}",
        adapter_symbol(&async_data.ffi_rust_future_complete.0)
    )?;
    Ok(())
}

fn render_haskell_ffi_import(out: &mut String, function: &general::Function) -> Result<()> {
    let callable = &function.callable;
    if let Some(async_data) = callable.async_data.as_ref() {
        return render_haskell_async_ffi_import(out, callable, async_data);
    }

    let mut parts = Vec::new();
    for argument in &callable.arguments {
        parts.push(haskell_ffi_argument_type(&argument.ty.ffi_type)?);
    }
    if let Some(return_type) = callable.return_type.ty.as_ref() {
        if is_by_value_struct(&return_type.ffi_type) {
            parts.push(format!(
                "Ptr {}",
                haskell_ffi_struct_type(&return_type.ffi_type)?
            ));
        }
    }
    parts.push("Ptr RustCallStatus".to_string());
    let return_type = match callable.return_type.ty.as_ref() {
        Some(return_type) if !is_by_value_struct(&return_type.ffi_type) => {
            haskell_ffi_scalar_type(&return_type.ffi_type)?
        }
        _ => "()".to_string(),
    };
    parts.push(format!("IO {return_type}"));
    writeln!(
        out,
        "foreign import ccall safe \"{}\" c_{} :: {}",
        adapter_symbol(&callable.ffi_func.0),
        haskell_value_name(&callable.name),
        parts.join(" -> ")
    )?;
    Ok(())
}

fn render_initialize(out: &mut String, namespace: &Namespace) -> Result<()> {
    writeln!(out, "initialization :: Initialization")?;
    writeln!(out, "initialization = unsafePerformIO newInitialization")?;
    writeln!(out, "{{-# NOINLINE initialization #-}}")?;
    writeln!(out)?;
    writeln!(out, "initialize :: IO ()")?;
    writeln!(
        out,
        "initialize = runInitialization initialization initializeOnce"
    )?;
    writeln!(out)?;
    writeln!(out, "initializeOnce :: IO ()")?;
    writeln!(out, "initializeOnce = do")?;
    writeln!(out, "  actualContract <- c_contractVersion")?;
    writeln!(
        out,
        "  unless (actualContract == {}) $",
        namespace.correct_contract_version
    )?;
    writeln!(
        out,
        "    throwIO (UniFFIException (Text.pack (\"UniFFI contract version mismatch: expected {}, got \" ++ show actualContract)))",
        namespace.correct_contract_version
    )?;
    for (index, checksum) in namespace.checksums.iter().enumerate() {
        writeln!(out, "  actualChecksum{index} <- c_checksum_{index}")?;
        writeln!(
            out,
            "  unless (actualChecksum{index} == {}) $",
            checksum.checksum
        )?;
        writeln!(
            out,
            "    throwIO (UniFFIException (Text.pack (\"UniFFI API checksum mismatch: expected {}, got \" ++ show actualChecksum{index})))",
            checksum.checksum
        )?;
    }
    for definition in &namespace.type_definitions {
        if let TypeDefinition::CallbackInterface(callback) = definition {
            writeln!(out, "  c_install{}", upper_camel(&callback.name))?;
        }
    }
    Ok(())
}

fn render_haskell_default_function(out: &mut String, function: &general::Function) -> Result<()> {
    let callable = &function.callable;
    let function_name = haskell_value_name(&callable.name);
    let required_arguments: Vec<_> = callable
        .arguments
        .iter()
        .filter(|argument| argument.default.is_none())
        .collect();
    let mut signature = required_arguments
        .iter()
        .map(|argument| haskell_argument_api_type(argument))
        .collect::<Result<Vec<_>>>()?;
    let result_type = callable
        .return_type
        .ty
        .as_ref()
        .map(|return_type| haskell_api_type(&return_type.ty))
        .transpose()?
        .unwrap_or_else(|| "()".to_string());
    let io_result_type = match callable.throws_type.ty.as_ref() {
        Some(error_type) => format!(
            "IO (Either {} {result_type})",
            haskell_api_type(&error_type.ty)?
        ),
        None => format!("IO ({result_type})"),
    };
    signature.push(io_result_type);
    writeln!(
        out,
        "{function_name}UsingDefaults :: {}",
        signature.join(" -> ")
    )?;
    let required_names: Vec<String> = required_arguments
        .iter()
        .map(|argument| haskell_value_name(&argument.name))
        .collect();
    if required_names.is_empty() {
        writeln!(out, "{function_name}UsingDefaults =")?;
    } else {
        writeln!(
            out,
            "{function_name}UsingDefaults {} =",
            required_names.join(" ")
        )?;
    }
    let call_arguments = callable
        .arguments
        .iter()
        .map(|argument| match &argument.default {
            Some(default) => render_default_value(default),
            None => Ok(haskell_value_name(&argument.name)),
        })
        .collect::<Result<Vec<_>>>()?;
    writeln!(out, "  {function_name} {}", call_arguments.join(" "))?;
    Ok(())
}

fn render_default_value(default: &general::DefaultValue) -> Result<String> {
    match default {
        general::DefaultValue::Literal(literal) => render_default_literal(literal),
        general::DefaultValue::Default(type_node) => default_for_type(&type_node.ty),
    }
}

fn render_default_literal(literal: &general::Literal) -> Result<String> {
    Ok(match literal {
        general::Literal::Boolean(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        general::Literal::String(value) => format!("Text.pack {value:?}"),
        general::Literal::UInt(value, _, _) => value.to_string(),
        general::Literal::Int(value, _, _) => value.to_string(),
        general::Literal::Float(value, _) => value.clone(),
        general::Literal::Enum(variant, type_node) => format!(
            "{}{}",
            haskell_api_type(&type_node.ty)?,
            upper_camel(variant)
        ),
        general::Literal::EmptySequence => "[]".to_string(),
        general::Literal::EmptyMap => "Map.empty".to_string(),
        general::Literal::EmptySet => "Set.empty".to_string(),
        general::Literal::None => "Nothing".to_string(),
        general::Literal::Some { inner } => {
            format!("Just ({})", render_default_value(inner)?)
        }
    })
}

fn default_for_type(ty: &Type) -> Result<String> {
    Ok(match ty {
        Type::UInt8
        | Type::Int8
        | Type::UInt16
        | Type::Int16
        | Type::UInt32
        | Type::Int32
        | Type::UInt64
        | Type::Int64
        | Type::Float32
        | Type::Float64 => "0".to_string(),
        Type::Boolean => "False".to_string(),
        Type::String => "Text.empty".to_string(),
        Type::Bytes => "mempty".to_string(),
        Type::Optional { .. } => "Nothing".to_string(),
        Type::Sequence { .. } => "[]".to_string(),
        Type::Map { .. } => "Map.empty".to_string(),
        Type::Set { .. } => "Set.empty".to_string(),
        Type::Duration => "Duration 0 0".to_string(),
        Type::Box { inner_type } => default_for_type(inner_type)?,
        Type::Custom { name, builtin, .. } => {
            format!("{} ({})", upper_camel(name), default_for_type(builtin)?)
        }
        Type::Record { name, .. } => format!("default{}", upper_camel(name)),
        other => bail!("default value for {other:?} is not supported yet"),
    })
}

fn render_haskell_function(out: &mut String, function: &general::Function) -> Result<()> {
    let callable = &function.callable;
    let function_name = haskell_value_name(&callable.name);
    let argument_names: Vec<String> = callable
        .arguments
        .iter()
        .map(|argument| haskell_value_name(&argument.name))
        .collect();
    let mut signature_parts: Vec<String> = callable
        .arguments
        .iter()
        .map(haskell_argument_api_type)
        .collect::<Result<_>>()?;
    let result_type = match callable.return_type.ty.as_ref() {
        Some(ty) => haskell_api_type(&ty.ty)?,
        None => "()".to_string(),
    };
    let io_result_type = match callable.throws_type.ty.as_ref() {
        Some(error_type) => format!(
            "IO (Either {} {result_type})",
            haskell_api_type(&error_type.ty)?
        ),
        None => format!("IO ({result_type})"),
    };
    signature_parts.push(io_result_type);
    writeln!(out, "{function_name} :: {}", signature_parts.join(" -> "))?;
    if argument_names.is_empty() {
        writeln!(out, "{function_name} = do")?;
    } else {
        writeln!(out, "{function_name} {} = do", argument_names.join(" "))?;
    }
    writeln!(out, "  initialize")?;

    for (argument, name) in callable.arguments.iter().zip(&argument_names) {
        if matches!(argument.ty.ty, Type::CallbackInterface { .. }) {
            writeln!(out, "  callback_{name}_handle <- newCallbackHandle {name}")?;
        }
        if let Type::Interface {
            name: interface_name,
            ..
        } = &argument.ty.ty
        {
            writeln!(
                out,
                "  object_{name}_handle <- clone{}Handle {name}",
                upper_camel(interface_name)
            )?;
        }
    }

    for (argument, name) in callable.arguments.iter().zip(&argument_names) {
        if matches!(argument.ty.ffi_type, FfiType::RustBuffer(_)) {
            let encoded = match &argument.ty.ty {
                Type::String => format!("encodeUtf8 {name}"),
                other => {
                    writeln!(
                        out,
                        "  let serialized_{name} = runEncoder ({})",
                        encoder_expression(other, name)?
                    )?;
                    format!("serialized_{name}")
                }
            };
            writeln!(
                out,
                "  lowered_{name} <- lowerRustBuffer c_rustBufferFromBytes c_rustBufferFree ({encoded})"
            )?;
        }
    }

    let mut indent = "  ".to_string();
    for (argument, name) in callable.arguments.iter().zip(&argument_names) {
        match &argument.ty.ffi_type {
            FfiType::RustBuffer(_) => {
                writeln!(
                    out,
                    "{indent}with lowered_{name} $ \\lowered_{name}_ptr -> do"
                )?;
                indent.push_str("  ");
            }
            FfiType::ForeignBytes => {
                let with_function = if is_mutable_borrowed_bytes(argument) {
                    "withMutableForeignBytes"
                } else {
                    "withForeignBytes"
                };
                writeln!(
                    out,
                    "{indent}{with_function} {name} $ \\borrowed_{name}_ptr -> do"
                )?;
                indent.push_str("  ");
            }
            _ => {}
        }
    }

    let mut ffi_arguments = Vec::new();
    for (argument, name) in callable.arguments.iter().zip(&argument_names) {
        ffi_arguments.push(lower_argument_expression(
            &argument.ty.ty,
            &argument.ty.ffi_type,
            name,
        )?);
    }

    if callable.async_data.is_some() {
        render_haskell_async_body(out, callable, &function_name, &ffi_arguments, &indent)?;
        return Ok(());
    }

    match callable.return_type.ty.as_ref() {
        Some(return_type) if is_by_value_struct(&return_type.ffi_type) => {
            writeln!(out, "{indent}alloca $ \\returnPtr -> do")?;
            let inner = format!("{indent}  ");
            writeln!(out, "{inner}poke returnPtr emptyRustBuffer")?;
            let args = append_call_arguments(&ffi_arguments, &["returnPtr", "statusPtr"]);
            writeln!(out, "{inner}(_, status) <-")?;
            writeln!(out, "{inner}  withRustCallStatus $ \\statusPtr ->")?;
            writeln!(out, "{inner}    c_{function_name} {args}")?;
            let success_indent = if let Some(error_type) = callable.throws_type.ty.as_ref() {
                writeln!(
                    out,
                    "{inner}callError <- checkRustCallStatusWithError c_rustBufferFree ({}) status",
                    decoder_expression(&error_type.ty)?
                )?;
                writeln!(out, "{inner}case callError of")?;
                writeln!(out, "{inner}  Just err -> pure (Left err)")?;
                writeln!(out, "{inner}  Nothing -> do")?;
                format!("{inner}    ")
            } else {
                writeln!(out, "{inner}checkRustCallStatus c_rustBufferFree status")?;
                inner.clone()
            };
            writeln!(out, "{success_indent}returnBuffer <- peek returnPtr")?;
            writeln!(
                out,
                "{success_indent}returnBytes <- consumeRustBuffer c_rustBufferFree returnBuffer"
            )?;
            render_lift_buffer(
                out,
                &success_indent,
                &return_type.ty,
                "returnBytes",
                callable.throws_type.ty.is_some(),
            )?;
        }
        return_type => {
            let args = append_call_arguments(&ffi_arguments, &["statusPtr"]);
            let return_binding = if return_type.is_some() {
                "returnValue"
            } else {
                "_"
            };
            writeln!(out, "{indent}({return_binding}, status) <-")?;
            writeln!(out, "{indent}  withRustCallStatus $ \\statusPtr ->")?;
            if args.is_empty() {
                writeln!(out, "{indent}    c_{function_name}")?;
            } else {
                writeln!(out, "{indent}    c_{function_name} {args}")?;
            }
            let success_indent = if let Some(error_type) = callable.throws_type.ty.as_ref() {
                writeln!(
                    out,
                    "{indent}callError <- checkRustCallStatusWithError c_rustBufferFree ({}) status",
                    decoder_expression(&error_type.ty)?
                )?;
                writeln!(out, "{indent}case callError of")?;
                writeln!(out, "{indent}  Just err -> pure (Left err)")?;
                writeln!(out, "{indent}  Nothing -> do")?;
                format!("{indent}    ")
            } else {
                writeln!(out, "{indent}checkRustCallStatus c_rustBufferFree status")?;
                indent.clone()
            };
            render_lift_scalar(
                out,
                &success_indent,
                return_type.map(|ty| &ty.ty),
                "returnValue",
                callable.throws_type.ty.is_some(),
            )?;
        }
    }
    Ok(())
}

fn render_haskell_async_body(
    out: &mut String,
    callable: &general::Callable,
    function_name: &str,
    ffi_arguments: &[String],
    indent: &str,
) -> Result<()> {
    let start_call = if ffi_arguments.is_empty() {
        format!("c_{function_name}_start")
    } else {
        format!("c_{function_name}_start {}", ffi_arguments.join(" "))
    };
    writeln!(out, "{indent}runRustFuture")?;
    writeln!(out, "{indent}  ({start_call})")?;
    writeln!(out, "{indent}  c_{function_name}_poll")?;
    writeln!(out, "{indent}  c_{function_name}_cancel")?;
    writeln!(out, "{indent}  c_{function_name}_free")?;
    writeln!(out, "{indent}  $ \\futureHandle -> do")?;
    let inner = format!("{indent}    ");

    match callable.return_type.ty.as_ref() {
        Some(return_type) if is_by_value_struct(&return_type.ffi_type) => {
            writeln!(out, "{inner}alloca $ \\returnPtr -> do")?;
            let complete_indent = format!("{inner}  ");
            writeln!(out, "{complete_indent}poke returnPtr emptyRustBuffer")?;
            writeln!(out, "{complete_indent}(_, status) <-")?;
            writeln!(
                out,
                "{complete_indent}  withRustCallStatus $ \\statusPtr ->"
            )?;
            writeln!(
                out,
                "{complete_indent}    c_{function_name}_complete futureHandle returnPtr statusPtr"
            )?;
            let success_indent = if let Some(error_type) = callable.throws_type.ty.as_ref() {
                writeln!(
                    out,
                    "{complete_indent}callError <- checkRustCallStatusWithError c_rustBufferFree ({}) status",
                    decoder_expression(&error_type.ty)?
                )?;
                writeln!(out, "{complete_indent}case callError of")?;
                writeln!(out, "{complete_indent}  Just err -> pure (Left err)")?;
                writeln!(out, "{complete_indent}  Nothing -> do")?;
                format!("{complete_indent}    ")
            } else {
                writeln!(
                    out,
                    "{complete_indent}checkRustCallStatus c_rustBufferFree status"
                )?;
                complete_indent.clone()
            };
            writeln!(out, "{success_indent}returnBuffer <- peek returnPtr")?;
            writeln!(
                out,
                "{success_indent}returnBytes <- consumeRustBuffer c_rustBufferFree returnBuffer"
            )?;
            render_lift_buffer(
                out,
                &success_indent,
                &return_type.ty,
                "returnBytes",
                callable.throws_type.ty.is_some(),
            )?;
        }
        return_type => {
            let return_binding = if return_type.is_some() {
                "returnValue"
            } else {
                "_"
            };
            writeln!(out, "{inner}({return_binding}, status) <-")?;
            writeln!(out, "{inner}  withRustCallStatus $ \\statusPtr ->")?;
            writeln!(
                out,
                "{inner}    c_{function_name}_complete futureHandle statusPtr"
            )?;
            let success_indent = if let Some(error_type) = callable.throws_type.ty.as_ref() {
                writeln!(
                    out,
                    "{inner}callError <- checkRustCallStatusWithError c_rustBufferFree ({}) status",
                    decoder_expression(&error_type.ty)?
                )?;
                writeln!(out, "{inner}case callError of")?;
                writeln!(out, "{inner}  Just err -> pure (Left err)")?;
                writeln!(out, "{inner}  Nothing -> do")?;
                format!("{inner}    ")
            } else {
                writeln!(out, "{inner}checkRustCallStatus c_rustBufferFree status")?;
                inner.clone()
            };
            render_lift_scalar(
                out,
                &success_indent,
                return_type.map(|return_type| &return_type.ty),
                "returnValue",
                callable.throws_type.ty.is_some(),
            )?;
        }
    }
    Ok(())
}

fn append_call_arguments(arguments: &[String], trailing: &[&str]) -> String {
    arguments
        .iter()
        .cloned()
        .chain(trailing.iter().map(|value| (*value).to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn lower_argument_expression(api_type: &Type, ffi_type: &FfiType, name: &str) -> Result<String> {
    match ffi_type {
        FfiType::RustBuffer(_) => return Ok(format!("lowered_{name}_ptr")),
        FfiType::ForeignBytes => return Ok(format!("borrowed_{name}_ptr")),
        _ => {}
    }
    Ok(match api_type {
        Type::Boolean => format!("(if {name} then 1 else 0)"),
        Type::Custom {
            name: type_name,
            builtin,
            ..
        } => {
            return lower_argument_expression(
                builtin,
                ffi_type,
                &format!("(unwrap{} {name})", upper_camel(type_name)),
            );
        }
        Type::Box { inner_type } => {
            return lower_argument_expression(inner_type, ffi_type, name);
        }
        Type::CallbackInterface { .. } => format!("callback_{name}_handle"),
        Type::Interface { .. } => format!("object_{name}_handle"),
        Type::UInt8
        | Type::Int8
        | Type::UInt16
        | Type::Int16
        | Type::UInt32
        | Type::Int32
        | Type::UInt64
        | Type::Int64
        | Type::Float32
        | Type::Float64 => name.to_string(),
        other => bail!("lowering {other:?} is not supported yet"),
    })
}

fn render_lift_buffer(
    out: &mut String,
    indent: &str,
    ty: &Type,
    value: &str,
    wrap_either: bool,
) -> Result<()> {
    match ty {
        Type::String => {
            writeln!(out, "{indent}case decodeUtf8 {value} of")?;
            writeln!(
                out,
                "{indent}  Left err -> throwIO (UniFFIException (Text.pack (\"invalid UTF-8 from Rust: \" ++ show err)))"
            )?;
            if wrap_either {
                writeln!(out, "{indent}  Right result -> pure (Right result)")?;
            } else {
                writeln!(out, "{indent}  Right result -> pure result")?;
            }
        }
        other if wrap_either => writeln!(
            out,
            "{indent}Right <$> runDecoder ({}) {value}",
            decoder_expression(other)?
        )?,
        other => writeln!(
            out,
            "{indent}runDecoder ({}) {value}",
            decoder_expression(other)?
        )?,
    }
    Ok(())
}

fn render_lift_scalar(
    out: &mut String,
    indent: &str,
    ty: Option<&Type>,
    value: &str,
    wrap_either: bool,
) -> Result<()> {
    match ty {
        None if wrap_either => writeln!(out, "{indent}pure (Right ())")?,
        None => writeln!(out, "{indent}pure ()")?,
        Some(Type::Boolean) => {
            writeln!(out, "{indent}case {value} of")?;
            if wrap_either {
                writeln!(out, "{indent}  0 -> pure (Right False)")?;
                writeln!(out, "{indent}  1 -> pure (Right True)")?;
            } else {
                writeln!(out, "{indent}  0 -> pure False")?;
                writeln!(out, "{indent}  1 -> pure True")?;
            }
            writeln!(
                out,
                "{indent}  other -> throwIO (UniFFIException (Text.pack (\"invalid boolean from Rust: \" ++ show other)))"
            )?;
        }
        Some(Type::Interface { name, .. }) if wrap_either => {
            let type_name = upper_camel(name);
            writeln!(
                out,
                "{indent}objectResult <- {type_name} <$> newRustObject {value} release{type_name}"
            )?;
            writeln!(out, "{indent}pure (Right objectResult)")?;
        }
        Some(Type::Interface { name, .. }) => {
            let type_name = upper_camel(name);
            writeln!(
                out,
                "{indent}{type_name} <$> newRustObject {value} release{type_name}"
            )?;
        }
        Some(Type::Custom { name, .. }) if wrap_either => {
            writeln!(out, "{indent}pure (Right ({} {value}))", upper_camel(name))?
        }
        Some(Type::Custom { name, .. }) => {
            writeln!(out, "{indent}pure ({} {value})", upper_camel(name))?
        }
        Some(Type::Box { inner_type }) => {
            return render_lift_scalar(out, indent, Some(inner_type), value, wrap_either);
        }
        Some(
            Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64
            | Type::Float32
            | Type::Float64,
        ) if wrap_either => writeln!(out, "{indent}pure (Right {value})")?,
        Some(
            Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64
            | Type::Float32
            | Type::Float64,
        ) => writeln!(out, "{indent}pure {value}")?,
        Some(other) => bail!("scalar lifting for {other:?} is not supported yet"),
    }
    Ok(())
}

fn is_mutable_borrowed_bytes(_argument: &general::Argument) -> bool {
    false
}

fn haskell_argument_api_type(argument: &general::Argument) -> Result<String> {
    if is_mutable_borrowed_bytes(argument) {
        Ok("MutableBytes".to_string())
    } else {
        haskell_api_type(&argument.ty.ty)
    }
}

fn haskell_api_type(ty: &Type) -> Result<String> {
    Ok(match ty {
        Type::UInt8 => "Word8".to_string(),
        Type::Int8 => "Int8".to_string(),
        Type::UInt16 => "Word16".to_string(),
        Type::Int16 => "Int16".to_string(),
        Type::UInt32 => "Word32".to_string(),
        Type::Int32 => "Int32".to_string(),
        Type::UInt64 => "Word64".to_string(),
        Type::Int64 => "Int64".to_string(),
        Type::Float32 => "Float".to_string(),
        Type::Float64 => "Double".to_string(),
        Type::Boolean => "Bool".to_string(),
        Type::String => "Text".to_string(),
        Type::Bytes => "ByteString".to_string(),
        Type::Optional { inner_type } => {
            format!("Maybe {}", parenthesized_haskell_type(inner_type)?)
        }
        Type::Sequence { inner_type } => format!("[{}]", haskell_api_type(inner_type)?),
        Type::Map {
            key_type,
            value_type,
        } => format!(
            "Map {} {}",
            haskell_type_argument(key_type)?,
            haskell_type_argument(value_type)?
        ),
        Type::Set { inner_type } => format!("Set {}", haskell_type_argument(inner_type)?),
        Type::Timestamp => "Timestamp".to_string(),
        Type::Duration => "Duration".to_string(),
        Type::Box { inner_type } => haskell_api_type(inner_type)?,
        Type::Interface { name, .. }
        | Type::CallbackInterface { name, .. }
        | Type::Record { name, .. }
        | Type::Enum { name, .. }
        | Type::Custom { name, .. } => upper_camel(name),
    })
}

fn haskell_type_argument(ty: &Type) -> Result<String> {
    let rendered = haskell_api_type(ty)?;
    if matches!(
        ty,
        Type::Optional { .. } | Type::Map { .. } | Type::Set { .. }
    ) {
        Ok(format!("({rendered})"))
    } else {
        Ok(rendered)
    }
}

fn parenthesized_haskell_type(ty: &Type) -> Result<String> {
    let rendered = haskell_api_type(ty)?;
    if matches!(
        ty,
        Type::Optional { .. } | Type::Map { .. } | Type::Set { .. }
    ) {
        Ok(format!("({rendered})"))
    } else {
        Ok(rendered)
    }
}

fn haskell_ffi_argument_type(ty: &FfiType) -> Result<String> {
    match ty {
        FfiType::RustBuffer(_) | FfiType::ForeignBytes => {
            Ok(format!("Ptr {}", haskell_ffi_struct_type(ty)?))
        }
        FfiType::Reference(inner) | FfiType::MutReference(inner) => {
            Ok(format!("Ptr {}", haskell_ffi_pointee_type(inner)?))
        }
        FfiType::VoidPointer => Ok("Ptr ()".to_string()),
        _ => haskell_ffi_scalar_type(ty),
    }
}

fn haskell_ffi_pointee_type(ty: &FfiType) -> Result<String> {
    match ty {
        FfiType::RustBuffer(_) | FfiType::ForeignBytes => haskell_ffi_struct_type(ty),
        FfiType::RustCallStatus => Ok("RustCallStatus".to_string()),
        FfiType::VoidPointer => Ok("()".to_string()),
        _ => haskell_ffi_scalar_type(ty),
    }
}

fn haskell_ffi_struct_type(ty: &FfiType) -> Result<String> {
    match ty {
        FfiType::RustBuffer(_) => Ok("RustBuffer".to_string()),
        FfiType::ForeignBytes => Ok("ForeignBytes".to_string()),
        other => bail!("Haskell FFI struct type for {other:?} is not supported yet"),
    }
}

fn haskell_ffi_scalar_type(ty: &FfiType) -> Result<String> {
    Ok(match ty {
        FfiType::UInt8 => "Word8".to_string(),
        FfiType::Int8 => "Int8".to_string(),
        FfiType::UInt16 => "Word16".to_string(),
        FfiType::Int16 => "Int16".to_string(),
        FfiType::UInt32 => "Word32".to_string(),
        FfiType::Int32 => "Int32".to_string(),
        FfiType::UInt64 | FfiType::Handle(_) => "Word64".to_string(),
        FfiType::Int64 => "Int64".to_string(),
        FfiType::Float32 => "Float".to_string(),
        FfiType::Float64 => "Double".to_string(),
        FfiType::VoidPointer => "Ptr ()".to_string(),
        other => bail!("Haskell FFI scalar type for {other:?} is not supported yet"),
    })
}

fn adapter_symbol(raw_symbol: &str) -> String {
    format!("hs_{raw_symbol}")
}

fn haskell_value_name(value: &str) -> String {
    let mut parts = value.split('_').filter(|part| !part.is_empty());
    let first = parts.next().unwrap_or("value").to_ascii_lowercase();
    let mut result = first;
    for part in parts {
        result.push_str(&upper_camel(part));
    }
    if is_haskell_keyword(&result) {
        result.push('_');
    }
    result
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

fn is_haskell_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "case"
            | "class"
            | "data"
            | "default"
            | "deriving"
            | "do"
            | "else"
            | "foreign"
            | "if"
            | "import"
            | "in"
            | "infix"
            | "infixl"
            | "infixr"
            | "instance"
            | "let"
            | "map"
            | "module"
            | "negate"
            | "newtype"
            | "of"
            | "qualified"
            | "then"
            | "type"
            | "where"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_haskell_shaped() {
        assert_eq!(haskell_value_name("roundtrip_string"), "roundtripString");
        assert_eq!(haskell_value_name("type"), "type_");
        assert_eq!(
            upper_camel("uniffi_haskell_fixture"),
            "UniffiHaskellFixture"
        );
    }
}
