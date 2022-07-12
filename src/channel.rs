use crate::*;
use concurrent_queue::ConcurrentQueue;
use event_listener::{Event, EventListener};
use std::sync::{
    atomic::{AtomicI32, AtomicUsize, Ordering},
};

/// Contains all data that should be shared between Addresses, Inboxes and the Child.
/// This is wrapped in an Arc to allow sharing between them.
pub struct Channel<M> {
    /// The underlying queue
    pub(crate) queue: ConcurrentQueue<M>,
    /// The capacity of the channel
    pub(crate) capacity: Capacity,

    /// The amount of addresses associated to this channel.
    /// Once this is 0, not more addresses can be created and the Channel is closed.
    pub(crate) sender_count: AtomicUsize,
    /// The amount of inboxes associated to this channel.
    /// Once this is 0, it is impossible to spawn new processes, and the Channel.
    /// has exited.
    pub(crate) receiver_count: AtomicUsize,

    /// Subscribe when trying to receive a message from this channel.
    pub(crate) recv_event: Event,
    /// Subscribe when trying to send a message into this channel.
    pub(crate) send_event: Event,
    /// Subscribe when waiting for Actor to exit.
    pub(crate) exit_event: Event,

    /// The amount of processes that should still be halted.
    /// Can be negative bigger than amount of processes in total.
    pub(crate) halt_count: AtomicI32,
}

impl<M> Channel<M> {
    /// Create a new channel, given an address count, inbox_count and capacity.
    ///
    /// It must be ensured that the correct amount of senders and receivers exist.
    pub fn new(sender_count: usize, receiver_count: usize, capacity: Capacity) -> Self {
        Self {
            queue: match &capacity {
                Capacity::Bounded(size) => ConcurrentQueue::bounded(*size),
                Capacity::Unbounded(_) => ConcurrentQueue::unbounded(),
            },
            capacity,
            sender_count: AtomicUsize::new(sender_count),
            receiver_count: AtomicUsize::new(receiver_count),
            recv_event: Event::new(),
            send_event: Event::new(),
            exit_event: Event::new(),
            halt_count: AtomicI32::new(0),
        }
    }

    /// Add an inbox to the channel, incrementing inbox-count by 1. Afterwards,
    /// a new Inbox may be created from this channel.
    ///
    /// Returns the previous inbox-count
    ///
    /// ## Panics
    /// * `prev-inbox-count == 0`
    pub fn add_receiver(&self) -> usize {
        let prev_count = self.receiver_count.fetch_add(1, Ordering::AcqRel);
        assert!(prev_count != 0);
        prev_count
    }

    /// Try to add an inbox to the channel, incrementing inbox-count by 1. Afterwards,
    /// a new Inbox may be created from this channel.
    ///
    /// Whereas `add_inbox()` panics if `prev-inbox-count == 0`, this method returns
    /// `Ok(prev inbox-count)` or `Err(())` instead.
    ///
    /// This method is slower than add_inbox, since it uses `fetch-update` on the inbox-count.
    pub fn try_add_receiver(&self) -> Result<usize, ()> {
        self.receiver_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |val| {
                if val < 1 {
                    None
                } else {
                    Some(val + 1)
                }
            })
            .map_err(|_| ())
    }

    /// Remove an Inbox from the channel, decrementing inbox-count by 1. This should be
    /// called during the Inbox's destructor.
    ///
    /// If there are no more Inboxes remaining, this will close the channel and set
    /// `inboxes_dropped` to true. This will also drop any messages still inside the
    /// channel.
    ///
    /// ## Notifies
    /// * `prev-inbox-count == 1` -> all exit-listeners
    ///
    /// ## Panics
    /// * `prev-inbox-count == 0`
    pub fn remove_receiver(&self) {
        // Subtract one from the inbox count
        let prev_count = self.receiver_count.fetch_sub(1, Ordering::AcqRel);
        assert!(prev_count != 0);

        // If previous count was 1, we have now exited
        if prev_count == 1 {
            self.close();
            // Also notify the exit-listeners, since the process exited.
            self.exit_event.notify(usize::MAX);
            // drop all messages, since no more inboxes exist.
            while self.queue.pop().is_ok() {}
        }
    }

    /// Add an Address to the channel, incrementing address-count by 1. Afterwards,
    /// a new Address may be created from this channel.
    ///
    /// Returns the previous address-count.
    pub fn add_sender(&self) -> usize {
        self.sender_count.fetch_add(1, Ordering::AcqRel)
    }

    /// Remove an Address from the channel, decrementing address-count by 1. This should
    /// be called from the destructor of the Address.
    ///
    /// Returns the previous address-count
    ///
    /// ## Notifies
    /// * `auto-close` && `prev-address-count == 1` -> all send_listeners & recv_listeners
    ///
    /// ## Panics
    /// * `prev-address-count == 0`
    pub fn remove_sender(&self) -> usize {
        // Subtract one from the inbox count
        let prev_address_count = self.sender_count.fetch_sub(1, Ordering::AcqRel);
        assert!(prev_address_count != 0);
        prev_address_count
    }

    /// Close the channel. Returns `true` if the channel was not closed before this.
    /// Otherwise, this returns `false`.
    ///
    /// ## Notifies
    /// * if `true` -> all send_listeners & recv_listeners
    pub fn close(&self) -> bool {
        if self.queue.close() {
            self.recv_event.notify(usize::MAX);
            self.send_event.notify(usize::MAX);
            true
        } else {
            false
        }
    }

    /// Can be called by an inbox to know whether it should halt.
    ///
    /// This decrements the halt-counter by one when it is called, therefore every
    /// inbox should only receive true from this method once! The inbox keeps it's own
    /// local state about whether it has received true from this method.
    pub fn should_halt(&self) -> bool {
        // todo: test this

        // If the count is bigger than 0, we might have to halt.
        if self.halt_count.load(Ordering::Acquire) > 0 {
            // Now subtract 1 from the count
            let prev_count = self.halt_count.fetch_sub(1, Ordering::AcqRel);
            // If the count before updating was bigger than 0, we halt.
            // If this decrements below 0, we treat it as if it's 0.
            if prev_count > 0 {
                return true;
            }
        }

        // Otherwise, just continue
        false
    }

    /// Halt n inboxes associated with this channel. If `n >= #inboxes`, all inboxes
    /// will be halted. This might leave `halt-count > inbox-count`, however that's not
    /// a problem. If n > i32::MAX, n = i32::MAX.
    ///
    /// # Notifies
    /// * `n` recv-listeners
    pub fn halt(&self, n: u32) {
        // todo: test this
        let n = i32::try_from(n).unwrap_or(i32::MAX);

        self.halt_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                // If the count < 0, act as if it's 0.
                if count < 0 {
                    Some(n)
                } else {
                    // Otherwise, add both together.
                    Some(count.saturating_add(n))
                }
            })
            .unwrap();

        self.recv_event.notify(n as usize);
    }

    /// Whether the queue asscociated to the channel has been closed.
    pub fn closed(&self) -> bool {
        self.queue.is_closed()
    }

    /// Returns the amount of messages currently in the channel.
    pub fn msg_count(&self) -> usize {
        self.queue.len()
    }

    /// Returns the amount of addresses this channel has.
    pub fn sender_count(&self) -> usize {
        self.sender_count.load(Ordering::Acquire)
    }

    /// Returns the amount of inboxes this channel has.
    pub fn receiver_count(&self) -> usize {
        self.receiver_count.load(Ordering::Acquire)
    }

    /// Capacity of the inbox.
    pub fn capacity(&self) -> &Capacity {
        &self.capacity
    }

    /// Get a new recv-event listener
    pub(crate) fn get_recv_listener(&self) -> EventListener {
        self.recv_event.listen()
    }

    /// Get a new send-event listener
    pub(crate) fn get_send_listener(&self) -> EventListener {
        self.send_event.listen()
    }

    /// Get a new exit-event listener todo!
    pub(crate) fn get_exit_listener(&self) -> EventListener {
        self.exit_event.listen()
    }
}

#[cfg(test)]
mod test {
    use crate::*;
    use event_listener::EventListener;
    use futures::FutureExt;
    use std::sync::Arc;

    #[test]
    fn capacity_types() {
        let channel = Channel::<()>::new(1, 1, Capacity::Bounded(10));
        assert!(channel.queue.capacity().is_some());

        let channel = Channel::<()>::new(1, 1, Capacity::Unbounded(BackPressure::default()));
        assert!(channel.queue.capacity().is_none());
    }

    #[test]
    fn closing() {
        let channel = Channel::<()>::new(1, 1, Capacity::default());
        let listeners = Listeners::size_10(&channel);

        channel.close();

        assert!(channel.closed());
        assert_eq!(channel.receiver_count(), 1);
        assert_eq!(channel.try_send(()), Err(TrySendError::Closed(())));
        assert_eq!(channel.try_recv(&mut true), Err(TryRecvError::Closed));
        listeners.assert_notified(Assert {
            recv: 10,
            exit: 0,
            send: 10,
        });
    }

    #[test]
    fn no_more_senders_remaining() {
        let channel = Channel::<()>::new(1, 1, Capacity::default());
        let listeners = Listeners::size_10(&channel);

        channel.remove_sender();

        assert!(!channel.closed());
        assert_eq!(channel.receiver_count(), 1);
        assert_eq!(channel.sender_count(), 0);
        assert_eq!(channel.try_send(()), Ok(()));
        assert_eq!(channel.try_recv(&mut true), Ok(()));
        listeners.assert_notified(Assert {
            recv: 1,
            exit: 0,
            send: 1,
        });
    }

    #[test]
    fn exiting() {
        let channel = Channel::<()>::new(1, 1, Capacity::default());
        let listeners = Listeners::size_10(&channel);

        channel.remove_receiver();

        assert!(channel.closed());
        assert_eq!(channel.receiver_count(), 0);
        assert_eq!(channel.sender_count(), 1);
        assert_eq!(channel.try_send(()), Err(TrySendError::Closed(())));
        assert_eq!(channel.try_recv(&mut true), Err(TryRecvError::Closed));
        listeners.assert_notified(Assert {
            recv: 10,
            exit: 10,
            send: 10,
        });
    }

    #[test]
    fn exiting_drops_all_messages() {
        let channel = Channel::<Arc<()>>::new(1, 1, Capacity::default());
        let msg = Arc::new(());
        channel.try_send(msg.clone()).unwrap();
        assert_eq!(Arc::strong_count(&msg), 2);
        channel.remove_receiver();
        assert_eq!(Arc::strong_count(&msg), 1);
    }

    #[test]
    fn closing_doesnt_drop_messages() {
        let channel = Channel::<Arc<()>>::new(1, 1, Capacity::default());
        let msg = Arc::new(());
        channel.try_send(msg.clone()).unwrap();
        assert_eq!(Arc::strong_count(&msg), 2);
        channel.close();
        assert_eq!(Arc::strong_count(&msg), 2);
    }

    #[test]
    fn try_add_inbox_with_0_receivers() {
        let channel = Channel::<Arc<()>>::new(1, 1, Capacity::default());
        channel.remove_receiver();
        assert_eq!(channel.try_add_receiver(), Err(()));
        assert_eq!(channel.receiver_count(), 0);
    }

    #[test]
    #[should_panic]
    fn add_inbox_with_0_receivers() {
        let channel = Channel::<Arc<()>>::new(1, 1, Capacity::default());
        channel.remove_receiver();
        channel.add_receiver();
    }

    #[test]
    fn sending_notifies() {
        let channel = Channel::<()>::new(1, 1, Capacity::default());
        let listeners = Listeners::size_10(&channel);
        channel.try_send(()).unwrap();

        listeners.assert_notified(Assert {
            recv: 1,
            exit: 0,
            send: 0,
        });
    }

    #[test]
    fn recveiving_notifies() {
        let channel = Channel::<()>::new(1, 1, Capacity::default());
        channel.try_send(()).unwrap();
        let listeners = Listeners::size_10(&channel);
        channel.try_recv(&mut true).unwrap();

        listeners.assert_notified(Assert {
            recv: 1,
            exit: 0,
            send: 1,
        });
    }

    struct Listeners {
        recv: Vec<EventListener>,
        exit: Vec<EventListener>,
        send: Vec<EventListener>,
    }

    struct Assert {
        recv: usize,
        exit: usize,
        send: usize,
    }

    impl Listeners {
        fn size_10<T>(channel: &Channel<T>) -> Self {
            Self {
                recv: (0..10)
                    .into_iter()
                    .map(|_| channel.get_recv_listener())
                    .collect(),
                exit: (0..10)
                    .into_iter()
                    .map(|_| channel.get_exit_listener())
                    .collect(),
                send: (0..10)
                    .into_iter()
                    .map(|_| channel.get_send_listener())
                    .collect(),
            }
        }

        fn assert_notified(self, assert: Assert) {
            let recv = self
                .recv
                .into_iter()
                .map(|l| l.now_or_never().is_some())
                .filter(|bool| *bool)
                .collect::<Vec<_>>()
                .len();
            let exit = self
                .exit
                .into_iter()
                .map(|l| l.now_or_never().is_some())
                .filter(|bool| *bool)
                .collect::<Vec<_>>()
                .len();
            let send = self
                .send
                .into_iter()
                .map(|l| l.now_or_never().is_some())
                .filter(|bool| *bool)
                .collect::<Vec<_>>()
                .len();

            assert_eq!(
                assert.recv, recv,
                "recv-notified: actual({}) vs asserted({})",
                recv, assert.recv
            );
            assert_eq!(
                assert.exit, exit,
                "exit-notified: actual({}) vs asserted({})",
                exit, assert.exit
            );
            assert_eq!(
                assert.send, send,
                "send-notified: actual({}) vs asserted({})",
                send, assert.send
            );
        }
    }
}
