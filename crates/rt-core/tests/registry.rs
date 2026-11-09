//! Registry (ADR 0002): explicit fill, hard errors on duplicates, read-only
//! lookups afterwards.

use core::any::TypeId;

use rt_core::{Object, RegistryBuilder, object_impl};

fn same_meta(found: Option<&rt_core::MetaObject>, expected: &rt_core::MetaObject) -> bool {
    found.is_some_and(|m| std::ptr::eq(m, expected))
}

#[derive(Object)]
struct Widget;
#[object_impl]
impl Widget {}

#[derive(Object)]
struct Button;
#[object_impl]
impl Button {}

mod shadowed {
    use rt_core::{Object, object_impl};

    #[derive(Object)]
    pub struct Button;
    #[object_impl]
    impl Button {}
}

#[test]
fn finds_registered_kinds_by_type_and_name() {
    let registry = RegistryBuilder::new()
        .register::<Widget>()
        .register::<Button>()
        .build();
    assert!(same_meta(registry.by_type_id(TypeId::of::<Widget>()), Widget::meta()));
    assert!(same_meta(registry.by_name("Widget"), Widget::meta()));
    assert!(same_meta(registry.by_name("Button"), Button::meta()));
    assert_eq!(registry.len(), 2);
}

#[test]
fn unregistered_kinds_miss() {
    let registry = RegistryBuilder::new().register::<Widget>().build();
    assert!(registry.by_type_id(TypeId::of::<Button>()).is_none());
    assert!(registry.by_name("Button").is_none());
    assert!(registry.by_name("nope").is_none());
}

#[test]
fn empty_registry_is_empty() {
    let registry = RegistryBuilder::new().build();
    assert!(registry.is_empty());
}

#[test]
#[should_panic(expected = "registered twice")]
fn duplicate_type_is_a_hard_error() {
    RegistryBuilder::new().register::<Widget>().register::<Widget>();
}

#[test]
#[should_panic(expected = "duplicate name")]
fn duplicate_name_is_a_hard_error() {
    RegistryBuilder::new()
        .register::<Button>()
        .register::<shadowed::Button>();
}

// The declarative composer (spec #36): `registry![...]` fills the builder
// with the same explicit chain, sealed read-only, with the same duplicate
// hard errors. A kind missing the Object trait fails to compile here, not
// later as a by-name lookup error.

#[test]
fn composer_registers_each_kind_and_seals() {
    let registry = rt_core::registry![Widget, Button];
    assert!(registry.by_name("Widget").is_some());
    assert!(registry.by_name("Button").is_some());
    assert!(same_meta(registry.by_name("Widget"), Widget::meta()));
    assert_eq!(registry.len(), 2);
}

#[test]
fn composer_duplicate_kind_is_a_hard_error() {
    let result = std::panic::catch_unwind(|| rt_core::registry![Widget, Widget]);
    assert!(result.is_err(), "duplicate kind registration panics at startup");
}

#[test]
fn composer_trailing_comma_is_accepted() {
    let registry = rt_core::registry![Widget,];
    assert_eq!(registry.len(), 1);
}
