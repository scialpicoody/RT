//! By-name connect/emit behind the default-off `dynamic-invoke` feature
//! (ADR 0005): a single entry point ([`Hub::dynamic`]), signature validation,
//! `Result` returns, and every lookup traced through the audit hook.

use core::any::{Any, TypeId};
use std::rc::Rc;

use crate::audit::TraceEvent;
use crate::handle::{ConnectionId, Handle};
use crate::meta::{MetaObject, SlotRoute};
use crate::property::CloneAny;
use crate::registry::Registry;
use crate::signal::Hub;

/// Failures of the by-name path; all carry the names involved.
#[derive(Debug, PartialEq, Eq)]
pub enum DynamicError {
    StaleHandle { name: &'static str },
    UnknownSignal { owner: &'static str, signal: &'static str },
    UnknownSlot { owner: &'static str, slot: &'static str },
    /// The type has no such property.
    UnknownProperty { owner: &'static str, property: &'static str },
    ArityMismatch { owner: &'static str, signal: &'static str, slot: &'static str },
    /// The payload's type did not match the declared type.
    PayloadMismatch { expected: &'static str },
}

/// The single by-name entry point: `hub.dynamic(&registry)` — exists only
/// under feature `dynamic-invoke`.
pub struct Dynamic<'a> {
    hub: &'a mut Hub,
    registry: &'a Registry,
}

impl Hub {
    /// Single entry to the by-name path (feature `dynamic-invoke`).
    #[cfg(feature = "dynamic-invoke")]
    pub fn dynamic<'a>(&'a mut self, registry: &'a Registry) -> Dynamic<'a> {
        Dynamic {
            hub: self,
            registry,
        }
    }
}

impl Dynamic<'_> {
    /// Resolve a live object's MetaObject through the registry, tracing the
    /// lookup.
    fn meta_of(
        &mut self,
        handle: Handle,
        name: &'static str,
    ) -> Result<(&'static MetaObject, TypeId), DynamicError> {
        let Some(type_id) = self.hub.type_of(handle) else {
            self.hub.trace_event(TraceEvent::DynamicLookup {
                name,
                resolved: false,
            });
            return Err(DynamicError::StaleHandle { name });
        };
        match self.registry.by_type_id(type_id) {
            Some(meta) => Ok((meta, type_id)),
            None => {
                self.hub.trace_event(TraceEvent::DynamicLookup {
                    name,
                    resolved: false,
                });
                Err(DynamicError::StaleHandle { name })
            }
        }
    }

    fn find_signal(
        &mut self,
        meta: &'static MetaObject,
        signal_name: &'static str,
    ) -> Result<(u32, usize, &'static crate::meta::SignalDescriptor), DynamicError> {
        let merged = meta.find_signal_merged(signal_name);
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: signal_name,
            resolved: merged.is_some(),
        });
        let (index, signal) = merged.ok_or(DynamicError::UnknownSignal {
            owner: meta.type_name,
            signal: signal_name,
        })?;
        Ok((index, signal.args.len(), signal))
    }

    /// Connect `emitter.signal_name` to `receiver.slot_name`, both resolved
    /// through the registry. Signal/slot arity is validated against the
    /// descriptors.
    pub fn connect_by_name(
        &mut self,
        emitter: Handle,
        signal_name: &'static str,
        receiver: Handle,
        slot_name: &'static str,
    ) -> Result<ConnectionId, DynamicError> {
        let (emitter_meta, emitter_type) = self.meta_of(emitter, signal_name)?;
        let (receiver_meta, receiver_type) = self.meta_of(receiver, slot_name)?;

        let (signal_index, signal_arity, _) = self.find_signal(emitter_meta, signal_name)?;

        let merged_slot = receiver_meta.find_slot_merged(slot_name);
        let invoker = receiver_meta.slot_invoker_merged(slot_name);
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: slot_name,
            resolved: invoker.is_some() && merged_slot.is_some(),
        });
        let Some((slot, up)) = merged_slot else {
            return Err(DynamicError::UnknownSlot {
                owner: receiver_meta.type_name,
                slot: slot_name,
            });
        };
        let Some(invoker) = invoker else {
            return Err(DynamicError::UnknownSlot {
                owner: receiver_meta.type_name,
                slot: slot_name,
            });
        };
        if signal_arity != slot.args.len() {
            return Err(DynamicError::ArityMismatch {
                owner: emitter_meta.type_name,
                signal: signal_name,
                slot: slot_name,
            });
        }

        // The invoker hands back the slot's return value; queued and direct
        // deliveries discard it, so adapt to the unit-returning erased call.
        // An inherited slot (level > 0) resolves its receiver through the
        // derived type's parent route before the owner's invoker downcasts.
        let route = SlotRoute {
            route: receiver_meta.parent_route,
            up,
        };
        let call: crate::signal::ErasedCall = Rc::new(move |recv, payload| {
            invoker(recv, payload, route);
        });
        Ok(self.hub.store_connection(
            emitter,
            (emitter_type, signal_index),
            receiver,
            receiver_type,
            call,
            false,
            None,
        ))
    }

    /// Emit `emitter.signal_name` with an erased payload. The payload's type
    /// is validated against the signal's declared payload — unit, the bare
    /// single argument, or the argument tuple, whatever the arity.
    pub fn emit_by_name(
        &mut self,
        emitter: Handle,
        signal_name: &'static str,
        payload: Box<dyn CloneAny>,
    ) -> Result<usize, DynamicError> {
        let (meta, emitter_type) = self.meta_of(emitter, signal_name)?;
        let (signal_index, _arity, signal) = self.find_signal(meta, signal_name)?;
        if (*payload.clone_any()).type_id() != signal.payload.id {
            return Err(DynamicError::PayloadMismatch {
                expected: signal.payload.name,
            });
        }

        Ok(self
            .hub
            .emit_erased(emitter, (emitter_type, signal_index), payload))
    }

    /// Invoke `receiver.slot_name` synchronously with an erased payload,
    /// returning the slot's value erased (`None` when the slot returns
    /// nothing). The payload's type is validated against the slot's declared
    /// payload; every lookup is traced.
    pub fn invoke_by_name(
        &mut self,
        receiver: Handle,
        slot_name: &'static str,
        payload: Box<dyn CloneAny>,
    ) -> Result<Option<Box<dyn Any>>, DynamicError> {
        let (meta, _) = self.meta_of(receiver, slot_name)?;

        let merged_slot = meta.find_slot_merged(slot_name);
        let invoker = meta.slot_invoker_merged(slot_name);
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: slot_name,
            resolved: merged_slot.is_some() && invoker.is_some(),
        });
        let Some((slot, up)) = merged_slot else {
            return Err(DynamicError::UnknownSlot {
                owner: meta.type_name,
                slot: slot_name,
            });
        };
        let Some(invoker) = invoker else {
            return Err(DynamicError::UnknownSlot {
                owner: meta.type_name,
                slot: slot_name,
            });
        };
        if (*payload.clone_any()).type_id() != slot.payload.id {
            return Err(DynamicError::PayloadMismatch {
                expected: slot.payload.name,
            });
        }
        let Some(value) = self.hub.any_mut(receiver) else {
            self.hub.trace_event(TraceEvent::DynamicLookup {
                name: slot_name,
                resolved: false,
            });
            return Err(DynamicError::StaleHandle { name: slot_name });
        };
        let route = SlotRoute {
            route: meta.parent_route,
            up,
        };
        Ok(invoker(&mut **value, payload.clone_any(), route))
    }

    /// Read `handle.property_name` erased. Every lookup is traced.
    pub fn get_property_by_name(
        &mut self,
        handle: Handle,
        property_name: &'static str,
    ) -> Result<Option<Box<dyn Any>>, DynamicError> {
        let (meta, _) = self.meta_of(handle, property_name)?;
        let descriptor = meta
            .find_property_merged(property_name)
            .map(|(_, descriptor)| descriptor);
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: property_name,
            resolved: descriptor.is_some(),
        });
        let Some(_) = descriptor else {
            return Err(DynamicError::UnknownProperty {
                owner: meta.type_name,
                property: property_name,
            });
        };
        let Some(props) = self.hub.property_inputs(handle) else {
            self.hub.trace_event(TraceEvent::DynamicLookup {
                name: property_name,
                resolved: false,
            });
            return Err(DynamicError::StaleHandle {
                name: property_name,
            });
        };
        Ok(props.get_property(property_name))
    }

    /// Write `handle.property_name` with an erased payload, validated against
    /// the declared property type. The write marks the property dirty —
    /// change signals deliver at the next flush, exactly like a typed `set`.
    /// Every lookup is traced.
    pub fn set_property_by_name(
        &mut self,
        handle: Handle,
        property_name: &'static str,
        value: Box<dyn CloneAny>,
    ) -> Result<(), DynamicError> {
        let (meta, _) = self.meta_of(handle, property_name)?;
        let descriptor = meta
            .find_property_merged(property_name)
            .map(|(_, descriptor)| descriptor);
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: property_name,
            resolved: descriptor.is_some(),
        });
        let Some(descriptor) = descriptor else {
            return Err(DynamicError::UnknownProperty {
                owner: meta.type_name,
                property: property_name,
            });
        };
        if (*value.clone_any()).type_id() != descriptor.ty.id {
            return Err(DynamicError::PayloadMismatch {
                expected: descriptor.ty.name,
            });
        }
        let Some(props) = self.hub.property_inputs(handle) else {
            self.hub.trace_event(TraceEvent::DynamicLookup {
                name: property_name,
                resolved: false,
            });
            return Err(DynamicError::StaleHandle {
                name: property_name,
            });
        };
        match props.set_property(property_name, value.clone_any()) {
            Ok(()) => Ok(()),
            // The descriptor check above makes this unreachable; treat it as
            // unknown rather than losing the invariant.
            Err(_) => Err(DynamicError::UnknownProperty {
                owner: meta.type_name,
                property: property_name,
            }),
        }
    }
}
