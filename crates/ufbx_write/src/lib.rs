#[expect(non_camel_case_types, non_snake_case, non_upper_case_globals)]
pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
