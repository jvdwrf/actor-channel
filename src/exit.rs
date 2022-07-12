use crate::*;
use el::{Event, EventListener};
use event_listener as el;
use futures::{Future, FutureExt};
use std::{
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

impl<M> Channel<M> {
    pub fn exit(&self) -> Exit<'_> {
        Exit::new(self)
    }

    pub fn exit_blocking(&self) {
        if !self.exited() {
            let listener = self.get_exit_listener();
            if !self.exited() {
                listener.wait();
                assert!(self.exited())
            }
        }
    }

    pub fn exited(&self) -> bool {
        self.receiver_count() == 0
    }
}

pub struct Exit<'a> {
    listener: Option<EventListener>,
    receiver_count: &'a AtomicUsize,
    exit_event: &'a Event,
}

impl<'a> Exit<'a> {
    pub(crate) fn new<M>(channel: &'a Channel<M>) -> Self {
        Self {
            listener: None,
            receiver_count: &channel.receiver_count,
            exit_event: &channel.exit_event,
        }
    }
}

impl<'a> Unpin for Exit<'a> {}

impl<'a> Future for Exit<'a> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        fn exited(receiver_count: &AtomicUsize) -> bool {
            receiver_count.load(Ordering::Acquire) == 0
        }

        if exited(&self.receiver_count) {
            self.listener = None;
            return Poll::Ready(());
        }

        if self.listener.is_none() {
            self.listener = Some(self.exit_event.listen());

            if exited(&self.receiver_count) {
                self.listener = None;
                return Poll::Ready(());
            }
        }

        match self.listener.as_mut().unwrap().poll_unpin(cx) {
            Poll::Ready(()) => {
                assert!(exited(&self.receiver_count)); // todo: remove this line
                self.listener = None;
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
