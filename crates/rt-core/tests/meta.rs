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
            payload: TypeRef {
                name: "i32",
                id: TypeId::of::<i32>(),
            },
        },
        SignalDescriptor {
            name: "stuck",
            args: &[],
            payload: TypeRef {
                name: "()",
                id: TypeId::of::<()>(),
            },
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
            payload: TypeRef {
                name: "()",
                id: TypeId::of::<()>(),
            },
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
            payload: TypeRef {
                name: "&str",
                id: TypeId::of::<&str>(),
            },
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
    parent: Element,
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
struct Toggle3D {
    parent: Button,
}

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
fn parent_members_merge_with_offset_indices() {
    // Own members keep their own-space indices.
    assert_eq!(Button::meta().signals.len(), 1);
    assert_eq!(
        Button::meta().find_signal_merged("toggled"),
        Some((
            0,
            &Button::meta().signals[0] as &rt_core::SignalDescriptor
        ))
    );

    // Inherited signal: the ancestor level appends after the younger
    // levels' signal counts.
    let (index, signal) = Toggle3D::meta()
        .find_signal_merged("toggled")
        .expect("inherited signal merges");
    assert_eq!(index, 0); // Toggle3D owns no signals
    assert_eq!(signal.name, "toggled");
    assert!(Toggle3D::meta().find_signal_merged("absent").is_none());

    // Inherited property: Button's whole signal+property space appends
    // before its own property indices.
    let (property_index, descriptor) = Toggle3D::meta()
        .find_property_merged("pressed")
        .expect("inherited property merges");
    assert_eq!(
        property_index,
        (Toggle3D::meta().signals.len()
            + Toggle3D::meta().properties.len()
            + Button::meta().signals.len()) as u32
    );
    assert_eq!(descriptor.name, "pressed");

    // Slots: Toggle3D has none anywhere on its chain; the resolved level is
    // its own when it did.
    assert!(Toggle3D::meta().find_slot_merged("anything").is_none());

    // A two-level chain still resolves the grandparent level's members with
    // the accumulated offsets (Element contributes no members).
    let (index_from_button, _) = Button::meta()
        .find_property_merged("pressed")
        .expect("own property merges");
    assert_eq!(index_from_button, Button::meta().signals.len() as u32);
}

// Factory emission (spec #36): `#[object(default = expr)]` and bare
// `#[object(default)]` produce a factory on the static MetaObject; kinds
// without a rule register none.

#[derive(Object)]
#[object(default = DefaultedSolo { hidden: true })]
struct DefaultedSolo {
    hidden: bool,
}

#[object_impl]
impl DefaultedSolo {}

#[derive(Object, Default)]
#[object(default)]
struct DefaultDefaulted {
    hits: u32,
}

#[object_impl]
impl DefaultDefaulted {}

#[derive(Object)]
struct NoDefault {
    value: u32,
}

#[object_impl]
impl NoDefault {}

#[test]
fn default_expression_becomes_a_factory() {
    let meta = DefaultedSolo::meta();
    let factory = meta.factory.clone().expect("default = expr emits a factory");
    let value = (factory.construct)();
    let solo = value
        .downcast_ref::<DefaultedSolo>()
        .expect("factory builds the derived type");
    assert!(solo.hidden, "expression seed state survives construction");
}

#[test]
fn bare_default_builds_via_default_impl() {
    let meta = DefaultDefaulted::meta();
    let factory = meta.factory.clone().expect("bare default emits a factory");
    let value = (factory.construct)();
    let obj = value
        .downcast_ref::<DefaultDefaulted>()
        .expect("factory builds the derived type");
    assert_eq!(obj.hits, 0, "Default::default zero state");
}

#[test]
fn no_rule_registers_no_factory() {
    assert!(NoDefault::meta().factory.is_none());
}
