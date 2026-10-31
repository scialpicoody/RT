# RT

RT is a Qt-style object model runtime in Rust: statically generated metaobjects, typed signals and slots, stateful properties, and a deterministic event loop.

## Language

**Signal**:
A named occurrence at a moment in time; every emission is delivered, never merged or coalesced.
_Avoid_: event, message, notification

**Property**:
Named state where only the latest value matters; intermediate values may be coalesced.
_Avoid_: state variable, binding

**Slot**:
A named operation an object offers in response to a signal.
_Avoid_: callback, handler

**MetaObject**:
The static description of a kind of object: its signals, slots, properties, and parent kind.
_Avoid_: reflection data, class info

**Registry**:
The runtime's read-only index from a kind of object to its MetaObject.
_Avoid_: type store, metatype system

**Flush**:
The single deterministic step in which queued emissions are delivered and dirty properties are evaluated.
_Avoid_: tick, update, refresh

**Handle**:
An object's unforgeable identity; stable, and detectably stale after the object is gone.
_Avoid_: pointer, reference
