pub(crate) use uniffi_bindgen::pipeline::general::{
    self, FfiDefinition, FfiFunction, FfiFunctionType, FfiStruct, FfiType, FieldsKind,
    Namespace as UniFfiNamespace, Type, TypeDefinition,
};

pub(crate) struct Bindings {
    pub namespaces: Vec<Namespace>,
}

pub(crate) struct Namespace {
    pub interface: UniFfiNamespace,
    pub module_segment: String,
    pub public_module: String,
    pub internal_module: String,
    pub c_stem: String,
}
