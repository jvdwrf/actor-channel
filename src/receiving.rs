use crate::*;
use concurrent_queue::PopError;
use event_listener::EventListener;
use futures::{Future, FutureExt, Stream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

impl<M> Channel<M> {
    pub fn try_recv(&self, check_for_halt: &mut bool) -> Result<M, TryRecvError> {
        if *check_for_halt && self.should_halt() {
            *check_for_halt = false;
            return Err(TryRecvError::Halted);
        }

        match self.queue.pop() {
            Ok(msg) => {
                self.send_event.notify(1);
                self.recv_event.notify(1);
                Ok(msg)
            }
            Err(e) => match e {
                PopError::Empty => Err(TryRecvError::Empty),
                PopError::Closed => Err(TryRecvError::Closed),
            },
        }
    }

    pub fn recv<'a>(&'a self, check_for_halt: &'a mut bool) -> Rcv<'a, M> {
        Rcv::new(self, check_for_halt)
    }
}

pub struct Rcv<'a, M> {
    channel: &'a Channel<M>,
    listener: Option<EventListener>,
    check_for_halt: &'a mut bool,
}

impl<'a, M> Rcv<'a, M> {
    fn new(channel: &'a Channel<M>, check_for_halt: &'a mut bool) -> Self {
        Self {
            channel,
            listener: None,
            check_for_halt,
        }
    }
}

impl<'a, M> Unpin for Rcv<'a, M> {}

/// This future will stay pollable after polling to completion. It can be used to
/// retrieve multiple completed values.
impl<'a, M> Future for Rcv<'a, M> {
    type Output = Result<M, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let rcv = &mut *self;

        loop {
            // Attempt to receive a message, and return if necessary
            match rcv.channel.try_recv(&mut rcv.check_for_halt) {
                Ok(msg) => {
                    rcv.listener = None;
                    return Poll::Ready(Ok(msg));
                }
                Err(e) => match e {
                    TryRecvError::Halted => {
                        rcv.listener = None;
                        *rcv.check_for_halt = false;
                        return Poll::Ready(Err(RecvError::Halted));
                    }
                    TryRecvError::Closed => {
                        rcv.listener = None;
                        return Poll::Ready(Err(RecvError::ClosedAndEmpty));
                    }
                    TryRecvError::Empty => (),
                },
            }

            // Otherwise, acquire a listener, if we don't have one yet
            if rcv.listener.is_none() {
                rcv.listener = Some(rcv.channel.get_recv_listener())
            }

            // And poll the future
            match rcv.listener.as_mut().unwrap().poll_unpin(cx) {
                Poll::Ready(()) => rcv.listener = None,
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<'a, M> Stream for Rcv<'a, M> {
    type Item = Result<M, Halted>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.poll_unpin(cx).map(|val| match val {
            Ok(msg) => Some(Ok(msg)),
            Err(e) => match e {
                RecvError::Halted => Some(Err(Halted)),
                RecvError::ClosedAndEmpty => None,
            },
        })
    }
}

/// This Inbox has been halted.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Hash)]
#[error("This inbox has been halted")]
pub struct Halted;

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum TryRecvError {
    Closed,
    Empty,
    Halted,
}

/// Error returned when receiving a message from an inbox.
/// Reasons can be:
/// * `Halted`: This Inbox has been halted and should now exit.
/// * `ClosedAndEmpty`: This Inbox is closed and empty, it can no longer receive new messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RecvError {
    /// This inbox has been halted and should now exit.
    Halted,
    /// This inbox has been closed, and contains no more messages. It was closed either because
    /// all addresses have been dropped, or because it was manually closed.
    ClosedAndEmpty,
}
