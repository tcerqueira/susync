# Overview
An util crate to complete futures through a handle. Its main purpose is to bridge async Rust and callback-based APIs.

Inspired on the `future_handles` crate.

The `susync` crate uses standard library channels under the hood. It uses thread-safe primitives but expects low contention,
so it uses a single `SpinMutex` for shared state.
By design handles are allowed to race to complete the future so it is ok to call `complete` on handle of a completed future.

## Examples

Channel-like API:
```rust
async fn func() -> Option<u32> {
    let (future, handle) = susync::create();
    func_with_callback(|res| {
        handle.complete(res);
    });
    future.await.ok()
}
```

Scoped API:
```rust
async fn func() -> Option<u32> {
    let future = susync::suspend(|handle| {
        func_with_callback(|res| {
            handle.complete(res);
        });
    });
    future.await.ok()
}
```