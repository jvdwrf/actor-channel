use crate::*;
use futures::{FutureExt, Stream};
use std::{any::Any, fmt::Debug, mem::ManuallyDrop, sync::Arc, task::Poll, time::Duration};
use tokio::task::JoinHandle;

/// A [ChildPool] is similar to a [Child], except that the `Actor` can have more
/// than one `Process`. A [ChildPool] can be streamed to get the exit-values of
/// all spawned `tokio::task`s.
pub struct ChannelOwner<E: Send + 'static, S: Haltable + ?Sized> {
    shared: Arc<S>,
    handles: Option<Vec<JoinHandle<E>>>,
    link: Link,
    is_aborted: bool,
}

impl<E: Send + 'static, S: Haltable + ?Sized> ChannelOwner<E, S> {
    pub fn new(shared: Arc<S>, handles: Vec<JoinHandle<E>>, link: Link, is_aborted: bool) -> Self {
        Self {
            shared,
            handles: Some(handles),
            link,
            is_aborted,
        }
    }

    pub fn shared(&self) -> &Arc<S> {
        &self.shared
    }

    /// Split the child into it's parts.
    ///
    /// This will not run the destructor, and therefore the child will not be notified.
    pub fn into_parts(self) -> (Arc<S>, Vec<JoinHandle<E>>, Link, bool) {
        let no_drop = ManuallyDrop::new(self);
        unsafe {
            let mut handle = std::ptr::read(&no_drop.handles);
            let channel = std::ptr::read(&no_drop.shared);
            let link = std::ptr::read(&no_drop.link);
            let is_aborted = std::ptr::read(&no_drop.is_aborted);
            (channel, handle.take().unwrap(), link, is_aborted)
        }
    }

    /// Abort the `Actor`.
    ///
    /// Returns `true` if this is the first abort.
    pub fn abort(&mut self) -> bool {
        let was_aborted = self.is_aborted;
        self.is_aborted = true;
        for handle in self.handles.as_ref().unwrap() {
            handle.abort()
        }
        !was_aborted
    }

    /// Attach the `Actor`. Returns the old abort-timeout, if it was attached before this.
    pub fn attach(&mut self, duration: Duration) -> Option<Duration> {
        self.link.attach(duration)
    }

    /// Detach the `Actor`. Returns the old abort-timeout, if it was attached before this.
    pub fn detach(&mut self) -> Option<Duration> {
        self.link.detach()
    }

    /// Get a reference to the current [Link] of the `Actor`.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Whether all `tokio::tasks` have exited.
    pub fn exited(&self) -> bool {
        self.handles
            .as_ref()
            .unwrap()
            .iter()
            .all(|handle| handle.is_finished())
    }

    /// The amount of `tokio::task`s that are still alive
    pub fn task_count(&self) -> usize {
        self.handles
            .as_ref()
            .unwrap()
            .iter()
            .filter(|handle| !handle.is_finished())
            .collect::<Vec<_>>()
            .len()
    }

    /// The amount of `Child`ren in this `ChildPool`, this includes both alive and
    /// dead `Processes`.
    pub fn child_count(&self) -> usize {
        self.handles.as_ref().unwrap().len()
    }

    /// Whether the `Actor` is aborted.
    pub fn is_aborted(&self) -> bool {
        self.is_aborted
    }

    pub fn push_handle(&mut self, handle: JoinHandle<E>) {
        self.handles.as_mut().unwrap().push(handle);
    }
}

impl<E: Send + 'static, T: Haltable + ?Sized> Stream for ChannelOwner<E, T> {
    type Item = Result<E, ExitError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.handles.as_ref().unwrap().len() == 0 {
            return Poll::Ready(None);
        }

        for (i, handle) in self.handles.as_mut().unwrap().iter_mut().enumerate() {
            if let Poll::Ready(res) = handle.poll_unpin(cx) {
                self.handles.as_mut().unwrap().swap_remove(i);
                return Poll::Ready(Some(res.map_err(Into::into)));
            }
        }

        Poll::Pending
    }
}

impl<E: Send + 'static, T: Haltable + ?Sized> Unpin for ChannelOwner<E, T> {}

impl<E: Send + 'static, T: Haltable + ?Sized> Drop for ChannelOwner<E, T> {
    fn drop(&mut self) {
        if let Link::Attached(abort_timer) = self.link {
            if !self.is_aborted && !self.exited() {
                if abort_timer.is_zero() {
                    self.abort();
                } else {
                    self.shared.halt(u32::MAX);
                    let handles = self.handles.take().unwrap();
                    tokio::task::spawn(async move {
                        tokio::time::sleep(abort_timer).await;
                        for handle in handles {
                            handle.abort()
                        }
                    });
                }
            }
        }
    }
}

//------------------------------------------------------------------------------------------------
//  Errors
//------------------------------------------------------------------------------------------------

/// An error returned from an exiting tokio-task.
///
/// Can be either because it has panicked, or because it was aborted.
#[derive(Debug, thiserror::Error)]
pub enum ExitError {
    #[error("Child has panicked")]
    Panic(Box<dyn Any + Send>),
    #[error("Child has been aborted")]
    Abort,
}

impl ExitError {
    /// Whether the error is a panic.
    pub fn is_panic(&self) -> bool {
        match self {
            ExitError::Panic(_) => true,
            ExitError::Abort => false,
        }
    }

    /// Whether the error is an abort.
    pub fn is_abort(&self) -> bool {
        match self {
            ExitError::Panic(_) => false,
            ExitError::Abort => true,
        }
    }
}

impl From<tokio::task::JoinError> for ExitError {
    fn from(e: tokio::task::JoinError) -> Self {
        match e.try_into_panic() {
            Ok(panic) => ExitError::Panic(panic),
            Err(_) => ExitError::Abort,
        }
    }
}

/// An error returned when trying to spawn more processes onto a `Channel`.
#[derive(Clone, thiserror::Error)]
pub enum TrySpawnError<T> {
    #[error("Channel has already exited")]
    Exited(T),
    #[error("Inbox type did not match")]
    IncorrectInboxType(T),
}

impl<T> Debug for TrySpawnError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TrySpawnError").finish()
    }
}