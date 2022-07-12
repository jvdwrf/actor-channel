use crate::{Capacity, Channel, Exit};
use std::any::Any;

/// A [Channel], with a it's message type-erased.
pub trait AnyChannel {
    /// See [Channel::close]
    fn close(&self) -> bool;
    /// See [Channel::closed]
    fn closed(&self) -> bool;
    /// See [Channel::halt]
    fn halt(&self, n: u32);
    /// See [Channel::capacity]
    fn capacity(&self) -> &Capacity;
    /// See [Channel::should_halt]
    fn should_halt(&self) -> bool;

    /// See [Channel::receiver_count]
    fn receiver_count(&self) -> usize;
    /// See [Channel::sender_count]
    fn sender_count(&self) -> usize;
    /// See [Channel::msg_count]
    fn msg_count(&self) -> usize;

    /// See [Channel::add_receiver]
    fn add_receiver(&self) -> usize;
    /// See [Channel::try_add_receiver]
    fn try_add_receiver(&self) -> Result<usize, ()>;
    /// See [Channel::remove_receiver]
    fn remove_receiver(&self);
    /// See [Channel::add_sender]
    fn add_sender(&self) -> usize;
    /// See [Channel::remove_sender]
    fn remove_sender(&self) -> usize;

    /// See [Channel::exited]
    fn exited(&self) -> bool;
    /// See [Channel::exit]
    fn exit(&self) -> Exit<'_>;
    /// See [Channel::exit_blocking]
    fn exit_blocking(&self);
}

impl<M: Send + 'static> AnyChannel for Channel<M> {
    fn close(&self) -> bool {
        self.close()
    }
    fn closed(&self) -> bool {
        self.closed()
    }
    fn halt(&self, n: u32) {
        self.halt(n)
    }
    fn capacity(&self) -> &Capacity {
        self.capacity()
    }
    fn should_halt(&self) -> bool {
        self.should_halt()
    }
    fn receiver_count(&self) -> usize {
        self.receiver_count()
    }
    fn sender_count(&self) -> usize {
        self.sender_count()
    }
    fn msg_count(&self) -> usize {
        self.msg_count()
    }
    fn add_receiver(&self) -> usize {
        self.add_receiver()
    }
    fn try_add_receiver(&self) -> Result<usize, ()> {
        self.try_add_receiver()
    }
    fn remove_receiver(&self) {
        self.remove_receiver()
    }
    fn add_sender(&self) -> usize {
        self.add_sender()
    }
    fn remove_sender(&self) -> usize {
        self.remove_sender()
    }
    fn exited(&self) -> bool {
        self.exited()
    }
    fn exit(&self) -> Exit<'_> {
        Exit::new(self)
    }
    fn exit_blocking(&self) {
        self.exit_blocking()
    }
}
