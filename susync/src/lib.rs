//! # Overview
//! An util crate to complete futures through a handle. Its main purpose is to bridge async Rust and callback-based APIs.
//!
//! Inspired on the `future_handles` crate.
//!
//! The `susync` crate uses standard library channels under the hood. It uses thread-safe primitives but expects low contention,
//! so it uses a single [`SpinMutex`] for shared state. It should also work on `no_std` environments but it is not tested.
//! By design handles are allowed to race to complete the future so it is ok to call complete on handle of a completed future. More info [here][`SuspendHandle::clone`].
//!
//! ## Examples
//!
//! Channel-like API:
//! ```rust
//! async fn func() -> Option<u32> {
//!     let (future, handle) = susync::create();
//!
//!     func_with_callback(|res| {
//!         handle.complete(res);
//!     });
//!
//!     future.await.ok()
//! }
//! # fn func_with_callback(func: impl FnOnce(u32)) {
//! #    func(1);
//! # }
//! ```
//!
//! Scoped API:
//! ```rust
//! async fn func() -> Option<u32> {
//!     let future = susync::suspend(|handle| {
//!         func_with_callback(|res| {
//!             handle.complete(res);
//!         });
//!     });
//!
//!     future.await.ok()
//! }
//! # fn func_with_callback(func: impl FnOnce(u32)) {
//! #    func(1);
//! # }
//! ```
//!
//! ## Thread safety
//!
//! Currently it uses thread safe primitives so keep in mind that overhead.
//!
//! # Danger!
//!
//! Do **NOT** do this, it will block forever!
//! ```rust
//! async fn func() {
//!     let (future, handle) = susync::create();
//!     // Start awaiting here...
//!     future.await.unwrap();
//!     // Now we'll never be able set the result!
//!     handle.complete(1);
//! }
//! ```
//! Awaiting a [`SuspendFuture`] before setting the result with
//! [`SuspendHandle`] will cause a **deadlock**!
//!
//! # Macro shortcuts
//!
//! If your use case is to simply to call `complete` on the [`SuspendHandle`] with the arguments of a callback, the [`sus`] macro is an option.
//!
//! ```rust
//! # use susync::sus;
//! # async fn func() {
//! // With one closure argument
//! fn func_one_arg(func: impl FnOnce(u32)) {
//!    func(42);
//! }
//! // Here the arguments of the closure will be passed to `complete`
//! let result = sus!(func_one_arg(|x| {})).await.unwrap();
//! assert_eq!(result, 42);
//!
//! // With two closure arguments
//! fn func_two_args(func: impl FnOnce(u32, f32)) {
//!    func(42, 69.0);
//! }
//! // Here the arguments of the closure will be passed to `complete`
//! let result = sus!(func_two_args(|x, y| {})).await.unwrap();
//! assert_eq!(result, (42, 69.0));
//! # }
//! ```
//!
//! The [`SuspendFuture`] will hold the arguments in a tuple or just a type in case it's just one argument.
//! You can ignore arguments by using the wildcard `_` token.
//!
//! ```rust
//! # use susync::sus;
//! # async fn func() {
//! fn func_with_callback(func: impl FnOnce(u32, &str)) {
//!    func(42, "ignored :(");
//! }
//! // Here the second argument gets ignored
//! let result = sus!(func_with_callback(|x, _| {})).await.unwrap();
//! assert_eq!(result, 42);
//! # }
//! ```
//!
//! ## Macro invariants
//!
//! The callback argument must be a closure.
//! This macro implementation revolves around generating a new closure that runs the original and also forwards the arguments to [`SuspendHandle::complete`].
//! The logic is wrapped in a [`suspend`] that returns a future.
//! For this reason it's not possible to accept anything else other than a closure because it's not possible to infer the future output type.
//!
//! The [`sus`] macro calls `to_owned` on all arguments so all arguments in the callback must implement [`ToOwned`] trait.
//! This is to allow reference arguments like `&str` and any other reference of a type that implements [`Clone`] (check `ToOwned` [implementors]).
//!
//! [implementors]: https://doc.rust-lang.org/std/borrow/trait.ToOwned.html#implementors
//!
//! Still looking for ways to overcome this limitation and give the user the freedom to choose how to complete the future.
//!
//! ```rust, compile_fail
//! # use susync::sus;
//! # async fn func() {
//! // Does *NOT* implement `ToOwned`
//! struct RefArg(i32);
//! fn func(f: impl FnOnce(&RefArg)) {
//!    f(&RefArg(42));
//! }
//! // *ILLEGAL*: Here the argument will clone a reference and try to outlive the scope
//! let RefArg(result) = sus!(func(|arg| {})).await.unwrap();
//! # }
//! ```
//!
//! Unfortunately the error message is not very friendly because the error is trait bounds but they are implicit to the macro.
//! So if you ever get an error message like below it is likely a reference is being passed to complete instead of an owned value.
//!
//! ```text
//! error[E0521]: borrowed data escapes outside of closure
//!   --> susync/src/lib.rs:127:22
//!    |
//! 13 | let RefArg(result) = sus!(func(|arg| {})).await.unwrap();
//!    |                      ^^^^^^^^^^^---^^^^^^
//!    |                      |          |
//!    |                      |          `arg` is a reference that is only valid in the closure body
//!    |                      `handle` declared here, outside of the closure body
//!    |                      `arg` escapes the closure body here
//!    |
//!    = note: this error originates in the macro `sus`
//! ```
//!
//! In case there are more than one callback argument the macro only generates the boilerplate for the last one in the argument list.
//! For no particular reason, just vague assumption that result callbacks come last.
//!
//! ```rust
//! # use susync::sus;
//! # async fn func() {
//! fn func_with_callback(func1: impl FnOnce(i32), func2: impl FnOnce(f32)) {
//!    func1(42);
//!    func2(69.0);
//! }
//! // Here the macro only applies to the last closure
//! let result = sus!(func_with_callback(|_i| {}, |f| {})).await.unwrap();
//! assert_eq!(result, 69.0);
//! # }
//! ```

pub use susync_macros::sus;

use spin::mutex::SpinMutex;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc, Weak,
    },
    task::{Poll, Waker},
};

/// Future to suspend execution until [`SuspendHandle`] completes.
#[derive(Debug)]
pub struct SuspendFuture<T> {
    receiver: Receiver<T>,
    waker: Arc<SpinMutex<Option<Waker>>>,
}

/// Handle to signal a [`SuspendFuture`] that a result is ready.
#[derive(Debug)]
pub struct SuspendHandle<T> {
    sender: Sender<T>,
    waker: Weak<SpinMutex<Option<Waker>>>,
}

use thiserror::Error;

/// The error returned by [`SuspendFuture`] in the case of the error variant.
#[derive(Debug, Error)]
pub enum SuspendError {
    /// The [`SuspendHandle`]s were dropped before completing.
    #[error("handles were dropped before completing")]
    DroppedBeforeComplete,
}

/// An ergonomic `Result` type to wrap standard `Result` with error [`SuspendError`].
pub type SuspendResult<T> = Result<T, SuspendError>;

impl<T> SuspendFuture<T> {
    fn update_waker(&self, waker: Waker) {
        *self.waker.lock() = Some(waker);
    }
}

impl<T: Send> Future for SuspendFuture<T> {
    type Output = SuspendResult<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        match self.receiver.try_recv() {
            Err(TryRecvError::Empty) => {
                self.update_waker(cx.waker().clone());
                Poll::Pending
            }
            Err(TryRecvError::Disconnected) => {
                Poll::Ready(Err(SuspendError::DroppedBeforeComplete))
            }
            Ok(value) => Poll::Ready(Ok(value)),
        }
    }
}

impl<T> SuspendHandle<T> {
    /// Try to complete the [`SuspendFuture`] with given result. It will signal the future that a result is ready.
    /// Returns `true` if successfully sends the value to the future. It does **not** guarantee that the future will resolve with that value,
    /// it is possible that other futures are racing to complete the future.
    ///
    /// # Example
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// let (future, handle1) = susync::create();
    /// let handle2 = handle1.clone();
    /// let handle3 = handle1.clone();
    /// // successfully sends the value to complete on both
    /// assert!(handle1.complete(1));
    /// assert!(handle2.complete(2));
    ///
    /// let result = future.await.unwrap();
    /// // unsuccessfully tries to complete a future that completed
    /// assert!(!handle3.complete(3));
    /// assert_eq!(result, 1);
    /// # });
    /// ```
    pub fn complete(self, result: T) -> bool {
        let successful = self.send_update(result);
        if let Some(waker) = self.waker.upgrade() {
            waker.lock().take();
        }
        successful
    }

    fn send_update(&self, update: T) -> bool {
        let mut successful = false;
        if let Some(waker) = self.waker.upgrade() {
            if self.sender.send(update).is_ok() {
                successful = true;
            }
            // Wake future
            if let Some(waker) = waker.lock().take() {
                waker.wake();
            }
        }
        successful
    }
}

impl<T> Clone for SuspendHandle<T> {
    /// Implemented clone to allow racing for completion of the future.
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// let future = susync::suspend(|handle| {
    ///     // create another handle to race
    ///     let inner_handle = handle.clone();
    ///     let ret = func_with_callback_and_return(|res| {
    ///         // race to completion
    ///         inner_handle.complete(res);
    ///     });
    ///     // race to completion
    ///     handle.complete(ret);
    /// });
    /// // await first result to arrive
    /// future.await.ok()
    /// # });
    /// # fn func_with_callback_and_return(func: impl FnOnce(u32)) -> u32 {
    /// #    func(1);
    /// #    1
    /// # }
    /// ```
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            waker: Weak::clone(&self.waker),
        }
    }
}

impl<T> Drop for SuspendHandle<T> {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.upgrade() {
            // Wake future
            if let Some(waker) = waker.lock().take() {
                waker.wake();
            }
        }
    }
}

/// Creates a channel-like pair of [`SuspendFuture`] and [`SuspendHandle`].
///
/// ```rust
/// async fn func() {
///     // create future and handle pair
///     let (future, handle) = susync::create();
///     // set a result
///     handle.complete(1);
///     // await the result
///     let res = future.await.unwrap();
///     assert_eq!(res, 1);
/// }
/// ```
pub fn create<T: Send>() -> (SuspendFuture<T>, SuspendHandle<T>) {
    let (tx, rx) = mpsc::channel();
    let waker = Arc::new(SpinMutex::new(None));
    (
        SuspendFuture {
            receiver: rx,
            waker: Arc::clone(&waker),
        },
        SuspendHandle {
            sender: tx,
            waker: Arc::downgrade(&waker),
        },
    )
}

/// Creates a [`SuspendFuture`] from a closure.
///
/// ```rust
/// async fn func() {
///     // create future
///     let future = susync::suspend(|handle| {
///         func_with_callback(|res| {
///             // set a result
///             handle.complete(res);
///         });
///     });     
///     // await the result
///     let res = future.await.unwrap();
/// }
///
/// # fn func_with_callback(func: impl FnOnce(u32)) {
/// #    func(1);
/// # }
/// ```
pub fn suspend<T: Send>(func: impl FnOnce(SuspendHandle<T>)) -> SuspendFuture<T> {
    let (fut, handle) = create();
    func(handle);
    fut
}

#[cfg(test)]
mod tests;
