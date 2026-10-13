# Tauri runtime ownership patch

`tauri-runtime-wry-2.11.4` is packaged from upstream revision
`5a882eccfda53a189ec076c79c4ad186f50db5ff`, retaining its MIT/Apache licenses.
The normalized Cargo manifest allows this crate to build outside the upstream
workspace. The remaining runtime crates stay pinned to that revision.

The source patch wraps `Context::main_thread` in an `Arc` and clones the atomic
stores extracted by the event handler. Tao's Linux
event-loop target contains an `Rc<RefCell<HashSet<WindowId>>>`; deriving Clone
over that target races its reference count when Tauri handles are cloned or
dropped concurrently. Sharing the whole main-thread context avoids cloning its
non-atomic internals. Native access still follows the existing main-thread
dispatch checks, and the event loop retains an owner through shutdown.

The original runtime aborts during the following display-dependent reproducer:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --example runtime-handle-stress
```

It exercises 400,000 concurrent native handle clones and drops without opening
Verenu's database or calling providers. CI runs it under Xvfb. Remove this vendor
patch when the pinned upstream runtime provides equivalent ownership safety.
