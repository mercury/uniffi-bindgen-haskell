uniffi::setup_scaffolding!("uniffi_haskell_external_fixture");

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ExternalRecord {
    pub name: String,
    pub value: i64,
}

#[uniffi::export]
pub fn make_external_record(name: String, value: i64) -> ExternalRecord {
    ExternalRecord { name, value }
}
