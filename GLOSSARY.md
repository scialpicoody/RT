# RT

RT is a Qt-style object model runtime in Rust: statically generated metaobjects, typed signals and slots, stateful properties, and a deterministic event loop.

## Language

**Hub**:
The single-threaded runtime object owning the objects, the connections, the delivery FIFO, the bindings, and the Flush step; all emission, connection, and delivery goes through it.
_Avoid_: bus, broker, dispatcher, context

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

**Widget**:
A composable unit of the interface; it owns its state, paints itself at its resolved rectangle, and answers input, and is composed in an owned tree (Row/Column containers are Widgets).
_Avoid_: control, component, element, view

**Layout**:
The deterministic pass that turns the Widget tree plus the viewport into resolved rectangles — flex rules via taffy, at most once per cycle, ordered before painting.
_Avoid_: measure pass, sizing, arrangement

**Presenter**:
The fixed bridge that owns the window thread and the loop thread, marshals window events into messages, and presents painted frames; `run_window` is its only entry.
_Avoid_: window manager, event pump, host, runner

**Binding**:
A Property whose value is computed from other Properties; dependencies are captured on evaluation and it is re-evaluated once per Flush in insertion order.
_Avoid_: computed value, reactive expression
