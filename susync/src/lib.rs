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

use std::{sync::{mpsc::{Receiver, Sender, TryRecvError, self}, Arc, Weak}, future::Future, pin::Pin, task::{Poll, Waker}};
use spin::mutex::SpinMutex;

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
            },
            Err(TryRecvError::Disconnected) => {
                Poll::Ready(Err(SuspendError::DroppedBeforeComplete))
            },
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
        Self { sender: self.sender.clone(), waker: Weak::clone(&self.waker) }
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
        }
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
pub fn suspend<T: Send>(func: impl FnOnce(SuspendHandle<T>)) -> SuspendFuture<T>
{
    let (fut, handle) = create();
    func(handle);
    fut
}

// This approach doesnt work: https://doc.rust-lang.org/reference/macros-by-example.html#forwarding-a-matched-fragment
#[allow(unused_macros)]
macro_rules! map_if_closure {
    ( $handle:ident, |$( $args:ident ),*| $body:expr ) => {
        |$($args)*| {
            $handle.complete(($($args),*));
            $body
        }
    };
    ( $handle:ident, $arg:expr ) => { $arg };
}

#[macro_export]
macro_rules! suspend {
    ( $func:ident($($args:expr),*) ) => {
        $crate::suspend(|_handle| {
            $func($(map_if_closure!(handle, $args)),*);
        })
    };
}

#[cfg(test)]
mod tests;
