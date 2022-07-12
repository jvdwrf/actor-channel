use crate::*;
use concurrent_queue::PushError;
use el::EventListener;
use event_listener as el;
use futures::{Future, FutureExt};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::time::Sleep;

impl<M> Channel<M> {
    /// Send a message at this moment. This ignores any [BackPressure].
    ///
    /// See [Address::send_now]
    pub fn send_now(&self, msg: M) -> Result<(), TrySendError<M>> {
        match self.queue.push(msg) {
            Ok(()) => {
                self.recv_event.notify(1);
                Ok(())
            }
            Err(e) => match e {
                PushError::Full(msg) => Err(TrySendError::Full(msg)),
                PushError::Closed(msg) => Err(TrySendError::Closed(msg)),
            },
        }
    }

    /// Send a message at this moment. This fails if [BackPressure] gives a timeout.
    ///
    /// See [Address::send_now]
    pub fn try_send(&self, msg: M) -> Result<(), TrySendError<M>> {
        match self.capacity() {
            Capacity::Bounded(_) => Ok(self.send_now(msg)?),
            Capacity::Unbounded(backoff) => match backoff.get_timeout(self.msg_count()) {
                Some(_) => Err(TrySendError::Full(msg)),
                None => Ok(self.send_now(msg)?),
            },
        }
    }

    /// Same as `send` except blocking.
    pub fn send_blocking(&self, mut msg: M) -> Result<(), SendError<M>> {
        match self.capacity() {
            Capacity::Bounded(_) => loop {
                msg = match self.send_now(msg) {
                    Ok(()) => {
                        return Ok(());
                    }
                    Err(TrySendError::Closed(msg)) => {
                        return Err(SendError(msg));
                    }
                    Err(TrySendError::Full(msg)) => msg,
                };

                self.get_send_listener().wait();
            },
            Capacity::Unbounded(backoff) => {
                let timeout = backoff.get_timeout(self.msg_count());
                if let Some(timeout) = timeout {
                    std::thread::sleep(timeout);
                }
                self.send_now(msg).map_err(|e| match e {
                    TrySendError::Full(_) => unreachable!("unbounded"),
                    TrySendError::Closed(msg) => SendError(msg),
                })
            }
        }
    }

    pub fn send(&self, msg: M) -> Snd<'_, M> {
        Snd::new(self, msg)
    }
}

/// The send-future, this can be `.await`-ed to send the message.
pub struct Snd<'a, M> {
    channel: &'a Channel<M>,
    msg: Option<M>,
    fut: Option<SndFut>,
}

/// Listener for a bounded channel, sleep for an unbounded channel.
enum SndFut {
    Listener(EventListener),
    Sleep(Pin<Box<Sleep>>), // todo: can this box be removed?
}

impl<'a, M> Snd<'a, M> {
    pub(crate) fn new(channel: &'a Channel<M>, msg: M) -> Self {
        Snd {
            channel,
            msg: Some(msg),
            fut: None,
        }
    }
}

impl<'a, M> Unpin for Snd<'a, M> {}

impl<'a, M> Future for Snd<'a, M> {
    type Output = Result<(), SendError<M>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        fn bounded_send<M>(
            pin: &mut Pin<&mut Snd<'_, M>>,
            cx: &mut Context<'_>,
        ) -> Poll<Result<(), SendError<M>>> {
            let mut msg = pin.msg.take().unwrap();
            loop {
                // Try to send a message into the channel, and return if possible
                msg = match pin.channel.send_now(msg) {
                    Ok(()) => {
                        return Poll::Ready(Ok(()));
                    }
                    Err(TrySendError::Closed(msg)) => {
                        return Poll::Ready(Err(SendError(msg)));
                    }
                    Err(TrySendError::Full(msg)) => msg,
                };

                // Otherwise, we create the future if it doesn't exist yet.
                if pin.fut.is_none() {
                    pin.fut = Some(SndFut::Listener(pin.channel.get_send_listener()))
                }

                if let SndFut::Listener(listener) = pin.fut.as_mut().unwrap() {
                    // Poll it once, and return if pending, otherwise we loop again.
                    match listener.poll_unpin(cx) {
                        Poll::Ready(()) => pin.fut = None,
                        Poll::Pending => {
                            pin.msg = Some(msg);
                            return Poll::Pending;
                        }
                    }
                } else {
                    unreachable!("Channel must be bounded")
                }
            }
        }

        fn push_msg_unbounded<M>(pin: &mut Pin<&mut Snd<'_, M>>) -> Poll<Result<(), SendError<M>>> {
            let msg = pin.msg.take().unwrap();
            match pin.channel.send_now(msg) {
                Ok(()) => Poll::Ready(Ok(())),
                Err(TrySendError::Closed(msg)) => Poll::Ready(Err(SendError(msg))),
                Err(TrySendError::Full(_msg)) => unreachable!(),
            }
        }

        match self.channel.capacity() {
            Capacity::Bounded(_) => bounded_send(&mut self, cx),
            Capacity::Unbounded(backpressure) => match &mut self.fut {
                Some(SndFut::Sleep(sleep_fut)) => match sleep_fut.poll_unpin(cx) {
                    Poll::Ready(()) => {
                        self.fut = None;
                        push_msg_unbounded(&mut self)
                    }
                    Poll::Pending => Poll::Pending,
                },
                None => match backpressure.get_timeout(self.channel.msg_count()) {
                    Some(timeout) => {
                        let mut sleep_fut = Box::pin(tokio::time::sleep(timeout));
                        match sleep_fut.poll_unpin(cx) {
                            Poll::Ready(()) => push_msg_unbounded(&mut self),
                            Poll::Pending => {
                                self.fut = Some(SndFut::Sleep(sleep_fut));
                                Poll::Pending
                            }
                        }
                    }
                    None => push_msg_unbounded(&mut self),
                },
                Some(SndFut::Listener(_)) => unreachable!("Channel must be unbounded"),
            },
        }
    }
}

/// An error returned when trying to send a message into a `Channel`, but not waiting for space.
///
/// This can be either because the `Channel` is closed, or because it is full.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrySendError<M> {
    Closed(M),
    Full(M),
}

/// An error returned when sending a message into a `Channel` because the `Channel` is closed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SendError<M>(pub M);
