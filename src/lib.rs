//! # Overview
//! An util crate to complete futures through a handle. Its main purpose is to bridge async Rust and callback-based APIs.
//! 
//! Inspired on the `future_handles` crate.
//! 
//! The `susync` crate uses standard library channels under the hood. It uses thread-safe primitives but expects low contention,
//! so it uses a single [`SpinMutex`] for shared state.
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
    receiver: Receiver<SuspendUpdate<T>>,
    waker: Arc<SpinMutex<Option<Waker>>>,
}

/// Handle to signal a [`SuspendFuture`] that a result is ready.
#[derive(Debug)]
pub struct SuspendHandle<T> {
    sender: Sender<SuspendUpdate<T>>,
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

enum SuspendUpdate<T> {
    Complete(T),
    HandleDropped,
    HandleCloned,
}

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
            Ok(update) => match update {
                SuspendUpdate::HandleDropped | SuspendUpdate::HandleCloned => {
                    let waker = cx.waker().clone();
                    waker.wake_by_ref();
                    self.update_waker(waker);
                    Poll::Pending
                },
                SuspendUpdate::Complete(res) => {
                    Poll::Ready(Ok(res))
                }
            },
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
        let successful = self.send_update(SuspendUpdate::Complete(result));
        if let Some(waker) = self.waker.upgrade() {
            waker.lock().take();
        }
        successful
    }

    fn send_update(&self, update: SuspendUpdate<T>) -> bool {
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
        self.send_update(SuspendUpdate::HandleCloned);
        Self { sender: self.sender.clone(), waker: Weak::clone(&self.waker) }
    }
}

impl<T> Drop for SuspendHandle<T> {
    fn drop(&mut self) {
        self.send_update(SuspendUpdate::HandleDropped);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn return_value() {
        let (fut, comp) = create();

        tokio::spawn(async move {
            let res = fut.await;
            assert!(res.is_ok());
        });

        tokio::spawn(async move {
            assert!(comp.complete(()));
        });
    }

    #[tokio::test]
    async fn drop_handle() {
        let (fut, comp) = create::<()>();
        
        let inner_comp = comp.clone();
        tokio::spawn(async move {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(inner_comp);
        });

        drop(comp);
        let res = fut.await;
        assert!(res.is_err(), "expected Err, got Ok result");
    }

    #[tokio::test]
    async fn complete_after_drop() {
        let (fut, comp) = create::<()>();
        
        let inner_comp = comp.clone();
        tokio::spawn(async move {
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert!(inner_comp.complete(()));
        });

        drop(comp);
        let res = fut.await;
        assert!(res.is_ok(), "expected Ok, got Err result");
    }

    #[tokio::test]
    async fn complete_after_await() {
        let (fut, comp) = create::<()>();
        
        let inner_comp = comp.clone();
        tokio::spawn(async move {
            std::thread::sleep(std::time::Duration::from_millis(100));
            inner_comp.complete(());
        });

        let res = fut.await;
        assert!(res.is_ok(), "expected Ok, got Err result");
        assert!(!comp.complete(()));
    }

    #[tokio::test]
    async fn many_drops_race() {
        let (fut, comp) = create::<()>();

        for _ in 0..10 {
            let _ = comp.clone();
        }
        drop(comp);

        let res= fut.await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn many_drops_race_async() {
        let (fut, comp) = create::<()>();

        for _ in 0..10 {
            let inner_comp = comp.clone();
            tokio::spawn(async move {
                drop(inner_comp);
            });
        }
        drop(comp);

        let res= fut.await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn many_handles_race() {
        let (fut, comp) = create::<()>();

        for i in 0..10 {
            let inner_comp = comp.clone();
            if i == 5 {
                assert!(inner_comp.complete(()));
            }
        }
        drop(comp);

        let res= fut.await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn many_handles_race_async() {
        let (fut, comp) = create::<()>();

        for i in 0..10 {
            let inner_comp = comp.clone();
            tokio::spawn(async move {
                if i == 5 {
                    assert!(inner_comp.complete(()));
                }
            });
        }
        drop(comp);

        let res= fut.await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn complete_before_await() {
        let (fut, comp) = create::<()>();

        assert!(comp.complete(()));
        tokio::spawn(async move {
            fut.await.unwrap();
        });
    }

    #[tokio::test]
    async fn complete_early_err() {
        let (fut, comp) = create();

        let inner_comp = comp.clone();
        let res = mock_callback_func_early_err(move |res| {
            assert!(!inner_comp.complete(Ok(res)));
        });
        if let Err(res) = res {
            assert!(comp.complete(Err(res)));
        }

        let res = fut.await;
        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(res.is_err());
    }

    fn mock_callback_func(cb: impl FnOnce(()) + Send + 'static) -> Result<(), ()> {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            cb(());
        });
        Ok(())
    }

    #[tokio::test]
    async fn suspend_func() {
        let fut = suspend(|comp| {
            let _ = mock_callback_func(move |res| {
                assert!(comp.complete(res));
            });
        });

        let res = fut.await;
        assert!(res.is_ok());
    }

    fn mock_callback_func_early_err(cb: impl FnOnce(()) + Send + 'static) -> Result<(), ()> {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            cb(());
        });
        Err(())
    }

    #[tokio::test]
    async fn suspend_func_early_err() {
        let fut = suspend(|comp| {
            let inner_comp = comp.clone();
            let res = mock_callback_func_early_err(move |res| {
                assert!(!inner_comp.complete(Ok(res)));
            });
            if let Err(res) = res {
                assert!(comp.complete(Err(res)));
            }
        });

        let res = fut.await;
        assert!(res.is_ok());
        let res = res.unwrap();
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn complete_multiple_sequencially() {
        let (future, handle1) = create();
        let handle2 = handle1.clone();
        let handle3 = handle1.clone();
        // successfully sends the value to complete on both
        assert!(handle1.complete(1));
        assert!(handle2.complete(2));
        
        let result = future.await.unwrap();
        // unsuccessfully tries to complete a future that completed
        assert!(!handle3.complete(3));
        assert_eq!(result, 1);
    }
}
