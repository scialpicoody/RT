//! By-name connect/emit behind the default-off `dynamic-invoke` feature
//! (ADR 0005): a single entry point ([`Hub::dynamic`]), signature validation,
//! `Result` returns, and every lookup traced through the audit hook.

use core::any::TypeId;
use std::rc::Rc;

use crate::audit::TraceEvent;
use crate::handle::{ConnectionId, Handle};
use crate::meta::MetaObject;
use crate::property::CloneAny;
use crate::registry::Registry;
use crate::signal::Hub;

/// Failures of the by-name path; all carry the names involved.
#[derive(Debug, PartialEq, Eq)]
pub enum DynamicError {
    StaleHandle { name: &'static str },
    UnknownSignal { owner: &'static str, signal: &'static str },
    UnknownSlot { owner: &'static str, slot: &'static str },
    ArityMismatch { owner: &'static str, signal: &'static str, slot: &'static str },
    /// The payload's type did not match the signal's declared argument.
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
    ) -> Result<(u32, usize), DynamicError> {
        let position = meta.signals.iter().position(|s| s.name == signal_name);
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: signal_name,
            resolved: position.is_some(),
        });
        let Some(position) = position else {
            return Err(DynamicError::UnknownSignal {
                owner: meta.type_name,
                signal: signal_name,
            });
        };
        let arity = meta.signals[position].args.len();
        Ok((position as u32, arity))
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

        let (signal_index, signal_arity) = self.find_signal(emitter_meta, signal_name)?;

        let invoker = receiver_meta
            .slot_invokers
            .iter()
            .find(|(name, _)| *name == slot_name)
            .map(|(_, invoker)| *invoker);
        let slot_arity = receiver_meta
            .slots
            .iter()
            .find(|s| s.name == slot_name)
            .map(|s| s.args.len());
        self.hub.trace_event(TraceEvent::DynamicLookup {
            name: slot_name,
            resolved: invoker.is_some(),
        });
        let Some(invoker) = invoker else {
            return Err(DynamicError::UnknownSlot {
                owner: receiver_meta.type_name,
                slot: slot_name,
            });
        };
        if Some(signal_arity) != slot_arity {
            return Err(DynamicError::ArityMismatch {
                owner: emitter_meta.type_name,
                signal: signal_name,
                slot: slot_name,
            });
        }

        let call: crate::signal::ErasedCall = Rc::new(invoker);
        Ok(self.hub.store_connection(
            emitter,
            (emitter_type, signal_index),
            receiver,
            receiver_type,
            call,
            false,
        ))
    }

    /// Emit `emitter.signal_name` with an erased payload. The payload's type
    /// is validated against the signal's declared argument (unit and
    /// single-argument signals; multi-argument payloads must be the exact
    /// tuple type — not checkable ahead, fails at delivery).
    pub fn emit_by_name(
        &mut self,
        emitter: Handle,
        signal_name: &'static str,
        payload: Box<dyn CloneAny>,
    ) -> Result<usize, DynamicError> {
        let (meta, emitter_type) = self.meta_of(emitter, signal_name)?;
        let (signal_index, arity) = self.find_signal(meta, signal_name)?;

        if arity <= 1 {
            let expected = if arity == 0 {
                TypeId::of::<()>()
            } else {
                meta.signals[signal_index as usize].args[0].id
            };
            let expected_name = if arity == 0 {
                "()"
            } else {
                meta.signals[signal_index as usize].args[0].name
            };
            if (*payload.clone_any()).type_id() != expected {
                return Err(DynamicError::PayloadMismatch {
                    expected: expected_name,
                });
            }
        }

        Ok(self
            .hub
            .emit_erased(emitter, (emitter_type, signal_index), payload))
    }
}
