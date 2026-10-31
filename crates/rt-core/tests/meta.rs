//! Golden descriptor tests for `#[derive(Object)]` static MetaObjects (ticket #3).
//! Fixture types exist for their shape; fields may stay unread.

#![allow(dead_code)]

use rt_core::{Object, Property, PropertyDescriptor, SignalDescriptor, SlotDescriptor, TypeRef, object_impl};
use std::any::TypeId;

#[derive(Object)]
struct Solo {
    hidden: bool,
}

#[object_impl]
impl Solo {}

#[test]
fn bare_type_has_minimal_meta() {
    let meta = Solo::meta();
    assert_eq!(meta.type_name, "Solo");
    assert!(meta.parent.is_none());
    assert!(meta.signals.is_empty());
    assert!(meta.slots.is_empty());
    assert!(meta.properties.is_empty());
}

#[derive(Object)]
struct Gauge {
    #[property]
    value: Property<i32>,
}

#[object_impl]
impl Gauge {
    #[signal]
    fn surged(&self, new_value: i32);

    #[signal]
    fn stuck(&self);

    fn helper(&self, delta: i32) -> i32 {
        delta + self.value()
    }
}

#[test]
fn signal_descriptors_match_declarations() {
    static EXPECTED: [SignalDescriptor; 2] = [
        SignalDescriptor {
            name: "surged",
            args: &[TypeRef {
                name: "i32",
                id: TypeId::of::<i32>(),
            }],
        },
        SignalDescriptor {
            name: "stuck",
            args: &[],
        },
    ];
    assert_eq!(Gauge::meta().signals, &EXPECTED);
}

#[object_impl]
impl Model {
    #[slot]
    fn clear(&mut self) {
        self.count.set(0);
        self.label.set(String::new());
    }

    #[slot]
    fn describe(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.label())
    }
}

#[test]
fn slot_descriptors_match_methods() {
    static EXPECTED: [SlotDescriptor; 2] = [
        SlotDescriptor {
            name: "clear",
            args: &[],
            ret: TypeRef {
                name: "()",
                id: TypeId::of::<()>(),
            },
        },
        SlotDescriptor {
            name: "describe",
            args: &[TypeRef {
                name: "&str",
                id: TypeId::of::<&str>(),
            }],
            ret: TypeRef {
                name: "String",
                id: TypeId::of::<String>(),
            },
        },
    ];
    assert_eq!(Model::meta().slots, &EXPECTED);
}

#[test]
fn slot_bodies_still_run() {
    let mut model = Model {
        count: Property::new(7),
        label: Property::new(String::from("red")),
        internal: 1,
    };
    assert_eq!(model.describe("color: "), "color: red");
    model.clear();
    assert_eq!(model.count(), 0);
    assert_eq!(model.describe("color: "), "color: ");
}

#[test]
fn plain_methods_are_not_described() {
    let meta = Gauge::meta();
    assert_eq!(meta.signals.len(), 2);
    assert!(meta.slots.is_empty());
}

#[derive(Object)]
struct Model {
    #[property]
    count: Property<i32>,
    #[property]
    label: Property<String>,
    internal: u8,
}

#[test]
fn property_descriptors_match_fields() {
    assert_eq!(
        Model::meta().properties,
        &[
            PropertyDescriptor {
                name: "count",
                ty: TypeRef {
                    name: "i32",
                    id: TypeId::of::<i32>(),
                },
            },
            PropertyDescriptor {
                name: "label",
                ty: TypeRef {
                    name: "String",
                    id: TypeId::of::<String>(),
                },
            },
        ],
    );
}

#[derive(Object)]
struct Element;

#[object_impl]
impl Element {}

#[derive(Object)]
#[object(parent = Element)]
struct Button {
    #[property]
    pressed: Property<bool>,
}

#[object_impl]
impl Button {
    #[signal]
    fn toggled(&self, now_pressed: bool);
}

#[derive(Object)]
#[object(parent = Button)]
struct Toggle3D;

#[object_impl]
impl Toggle3D {}

#[test]
fn parent_link_resolves_to_parent_meta() {
    let parent = Button::meta().parent_meta().expect("button has a parent");
    assert_eq!(parent.type_name, "Element");
    assert!(parent.signals.is_empty());
    assert!(std::ptr::eq(parent, Element::meta()));
}

#[test]
fn parent_chain_walks_to_the_root() {
    let parent = Toggle3D::meta()
        .parent_meta()
        .expect("3d toggle has a parent");
    assert_eq!(parent.type_name, "Button");
    let grandparent = parent.parent_meta().expect("button has a parent");
    assert_eq!(grandparent.type_name, "Element");
    assert!(grandparent.parent_meta().is_none());
    assert!(std::ptr::eq(grandparent, Element::meta()));
}

#[test]
fn parent_link_does_not_merge_members() {
    assert_eq!(Button::meta().signals.len(), 1);
    assert!(Element::meta().signals.is_empty());
    assert!(Toggle3D::meta().properties.is_empty());
}
