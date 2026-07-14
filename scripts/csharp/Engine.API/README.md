# Engine.API native boundary

`Engine.API` is a direct P/Invoke surface for a managed runtime hosted **inside
the Rust engine process**. Before managed code calls it, the Rust host must
install its versioned `FfiRegistry` callback table into the loaded
`engine_ffi` native library. The native library and host reject ABI version or
callback-table size mismatches.

`ProcessHost` launches scripts in a separate operating-system process. That
process does not share the engine's `WorldSlot`, component table, registry
statics, pointers, or callback lifetimes. Code running under `ProcessHost` must
not use these direct P/Invoke declarations; it must use the ProcessHost IPC
protocol. Loading a second `engine_ffi` DLL in that child process cannot make
the engine world available.

Component `Get`/`Set` uses a length-then-buffer UTF-8 JSON protocol. Only
component types exposed by the active runtime with both serialize and
deserialize hooks can be registered. A type name alone does not make a native
component script-serializable.

Coroutines use a versioned managed descriptor rather than passing a raw
`IEnumerator` pointer. After `Coroutine.Start` returns a valid handle, the
native main-thread scheduler owns the managed `GCHandle`; natural completion,
`Coroutine.Stop`, callback failure, scene/runtime replacement, and shutdown
all release it exactly once. Managed `MoveNext`, `WaitUntil`, and `WaitForAll`
callbacks catch exceptions before returning across the C ABI. `WaitForSeconds`
uses the engine frame delta, and `WaitForAsync` observes the host async handle
state after main-thread callback dispatch.
