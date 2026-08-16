use std::{
    collections::{HashMap, HashSet},
    future::poll_fn,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    task::Poll,
    time::{Duration, SystemTime},
};
use uniffi_haskell_external_fixture::ExternalRecord;

uniffi::setup_scaffolding!("uniffi_haskell_fixture");

#[derive(Clone, uniffi::Record)]
pub struct Person {
    pub name: String,
    pub age: u8,
    pub nickname: Option<String>,
    pub scores: Vec<i32>,
    pub avatar: Vec<u8>,
}

#[derive(Clone, uniffi::Enum)]
pub enum Status {
    Idle,
    Message { message: String },
    Detailed(u32, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserId(pub u64);
uniffi::custom_newtype!(UserId, u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label(pub String);
uniffi::custom_newtype!(Label, String);

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Tree {
    Leaf(i32),
    Node { left: Box<Tree>, right: Box<Tree> },
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
#[uniffi(name = "RenamedRecord")]
pub struct RecordToRename {
    #[uniffi(name = "renamed_field")]
    pub field_to_rename: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DefaultsRecord {
    #[uniffi(default)]
    pub boolean: bool,
    #[uniffi(default = 42)]
    pub integer: i32,
    #[uniffi(default = None)]
    pub optional_string: Option<String>,
    #[uniffi(default)]
    pub strings: Vec<String>,
    #[uniffi(default)]
    pub map: HashMap<String, String>,
    #[uniffi(default)]
    pub set: HashSet<String>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum TestError {
    #[error("division by zero")]
    DivisionByZero,
    #[error("invalid division: {message}")]
    InvalidDivision { message: String },
    #[error("negative divisor: {0}")]
    NegativeDivisor(u32),
    #[error("unexpected callback error: {message}")]
    UnexpectedCallback { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for TestError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::UnexpectedCallback {
            message: error.reason,
        }
    }
}

#[uniffi::export(callback_interface)]
pub trait TestCallback: Send + Sync {
    fn transform(&self, value: i32) -> i32;
    fn describe(&self, value: String) -> String;
    fn fallible(&self, value: u32) -> Result<u32, TestError>;
}

#[uniffi::export]
pub fn invoke_callback_transform(callback: Box<dyn TestCallback>, value: i32) -> i32 {
    callback.transform(value)
}

#[uniffi::export]
pub fn invoke_callback_describe(callback: Box<dyn TestCallback>, value: String) -> String {
    callback.describe(value)
}

#[uniffi::export]
pub fn invoke_callback_fallible(
    callback: Box<dyn TestCallback>,
    value: u32,
) -> Result<u32, TestError> {
    callback.fallible(value)
}

#[uniffi::export]
pub fn invoke_callback_concurrently(callback: Box<dyn TestCallback>, values: Vec<i32>) -> Vec<i32> {
    std::thread::scope(|scope| {
        let jobs = values
            .into_iter()
            .map(|value| {
                let callback = &callback;
                scope.spawn(move || callback.transform(value))
            })
            .collect::<Vec<_>>();
        jobs.into_iter()
            .map(|job| job.join().expect("callback thread panicked"))
            .collect()
    })
}

#[derive(uniffi::Object)]
pub struct Counter {
    value: AtomicI64,
}

#[uniffi::export]
impl Counter {
    #[uniffi::constructor]
    pub fn new(initial: i64) -> Self {
        Self {
            value: AtomicI64::new(initial),
        }
    }

    pub fn add(&self, delta: i64) -> i64 {
        self.value
            .fetch_add(delta, Ordering::SeqCst)
            .wrapping_add(delta)
    }

    pub fn get(&self) -> i64 {
        self.value.load(Ordering::SeqCst)
    }

    pub fn sum_bytes(&self, value: &[u8]) -> u32 {
        value.iter().map(|byte| u32::from(*byte)).sum()
    }

    pub fn roundtrip_person(&self, person: Person) -> Person {
        person
    }

    pub fn fallible_get(&self, should_fail: bool) -> Result<i64, TestError> {
        if should_fail {
            Err(TestError::DivisionByZero)
        } else {
            Ok(self.get())
        }
    }

    pub async fn async_get(&self) -> i64 {
        yield_once().await;
        self.get()
    }
}

#[uniffi::export]
pub fn roundtrip_external_record(value: ExternalRecord) -> ExternalRecord {
    value
}

#[uniffi::export]
pub fn roundtrip_counter(value: Arc<Counter>) -> Arc<Counter> {
    value
}

#[uniffi::export]
pub fn roundtrip_person(value: Person) -> Person {
    value
}

#[uniffi::export]
pub fn roundtrip_status(value: Status) -> Status {
    value
}

#[uniffi::export]
pub fn roundtrip_optional_person(value: Option<Person>) -> Option<Person> {
    value
}

#[uniffi::export]
pub fn roundtrip_people(value: Vec<Person>) -> Vec<Person> {
    value
}

#[uniffi::export]
pub fn roundtrip_strings(value: Vec<String>) -> Vec<String> {
    value
}

#[uniffi::export]
pub fn roundtrip_hash_map(value: HashMap<String, i64>) -> HashMap<String, i64> {
    value
}

#[uniffi::export]
pub fn roundtrip_hash_set(value: HashSet<String>) -> HashSet<String> {
    value
}

#[uniffi::export]
pub fn roundtrip_system_time(value: SystemTime) -> SystemTime {
    value
}

#[uniffi::export]
pub fn roundtrip_duration(value: Duration) -> Duration {
    value
}

#[uniffi::export]
pub fn roundtrip_user_id(value: UserId) -> UserId {
    value
}

#[uniffi::export]
pub fn roundtrip_label(value: Label) -> Label {
    value
}

#[uniffi::export]
pub fn roundtrip_tree(value: Tree) -> Tree {
    value
}

#[uniffi::export]
pub fn person_name(person: &Person) -> String {
    person.name.clone()
}

#[uniffi::export(name = "renamed_function")]
pub fn function_to_rename(value: RecordToRename) -> RecordToRename {
    value
}

#[uniffi::export]
pub fn roundtrip_defaults_record(value: DefaultsRecord) -> DefaultsRecord {
    value
}

#[uniffi::export(default(value = 21))]
pub fn double_with_default(value: i32) -> i32 {
    value + value
}

#[uniffi::export]
pub fn divide(dividend: i32, divisor: i32) -> Result<i32, TestError> {
    if divisor == 0 {
        Err(TestError::DivisionByZero)
    } else if dividend == i32::MIN && divisor == -1 {
        Err(TestError::InvalidDivision {
            message: "integer overflow".to_string(),
        })
    } else if divisor < 0 {
        Err(TestError::NegativeDivisor(divisor.unsigned_abs()))
    } else {
        Ok(dividend / divisor)
    }
}

async fn yield_once() {
    let mut first_poll = true;
    poll_fn(move |context| {
        if first_poll {
            first_poll = false;
            context.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await
}

#[uniffi::export]
pub async fn async_never() {
    poll_fn(|_| Poll::<()>::Pending).await;
}

#[uniffi::export]
pub async fn async_ping() {
    yield_once().await;
}

#[uniffi::export]
pub async fn async_add(a: u32, b: u32) -> u32 {
    yield_once().await;
    a + b
}

#[uniffi::export]
pub async fn async_roundtrip_person(value: Person) -> Person {
    yield_once().await;
    value
}

#[uniffi::export]
pub async fn async_divide(dividend: i32, divisor: i32) -> Result<i32, TestError> {
    yield_once().await;
    divide(dividend, divisor)
}

#[uniffi::export]
pub fn ping() {}

#[uniffi::export]
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[uniffi::export]
pub fn roundtrip_u8(value: u8) -> u8 {
    value
}

#[uniffi::export]
pub fn roundtrip_i8(value: i8) -> i8 {
    value
}

#[uniffi::export]
pub fn roundtrip_u16(value: u16) -> u16 {
    value
}

#[uniffi::export]
pub fn roundtrip_i16(value: i16) -> i16 {
    value
}

#[uniffi::export]
pub fn roundtrip_u32(value: u32) -> u32 {
    value
}

#[uniffi::export]
pub fn roundtrip_i32(value: i32) -> i32 {
    value
}

#[uniffi::export]
pub fn roundtrip_u64(value: u64) -> u64 {
    value
}

#[uniffi::export]
pub fn roundtrip_i64(value: i64) -> i64 {
    value
}

#[uniffi::export]
pub fn roundtrip_f32(value: f32) -> f32 {
    value
}

#[uniffi::export]
pub fn roundtrip_f64(value: f64) -> f64 {
    value
}

#[uniffi::export]
pub fn roundtrip_bool(value: bool) -> bool {
    value
}

#[uniffi::export]
pub fn roundtrip_string(value: String) -> String {
    value
}

#[uniffi::export]
pub fn roundtrip_bytes(value: Vec<u8>) -> Vec<u8> {
    value
}

#[uniffi::export]
pub fn sum_bytes(value: &[u8]) -> u32 {
    value.iter().map(|byte| u32::from(*byte)).sum()
}

#[uniffi::export]
pub fn panic_now() {
    panic!("fixture panic");
}

#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn sum_mixed_primitives(
    a: u8,
    b: i8,
    c: u16,
    d: i16,
    e: u32,
    f: i32,
    g: u64,
    h: i64,
    i: f32,
    j: f64,
    negate: bool,
) -> f64 {
    let sum = a as f64
        + b as f64
        + c as f64
        + d as f64
        + e as f64
        + f as f64
        + g as f64
        + h as f64
        + i as f64
        + j;

    if negate {
        -sum
    } else {
        sum
    }
}
