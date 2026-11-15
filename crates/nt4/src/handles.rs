//! Publisher, subscriber and event handles, shared by [`Client`](crate::Client)
//! and [`Server`](crate::Server).
//!
//! Each handle holds the runtime it came from through a small internal
//! trait, so the same types work with both.

use std::marker::PhantomData;
use std::sync::Arc;

use nt4_core::{ClientError, PubId, SubId, TopicValue};
use nt4_proto::Value;
use tokio::sync::{mpsc, watch};

use crate::NtValue;

/// What a handle needs from the client or server that made it.
pub(crate) trait Backend: Send + Sync {
    /// The runtime's clock, in microseconds, never 0.
    fn now_us(&self) -> i64;
    fn set_value(&self, id: PubId, value: Value, time_us: i64) -> Result<(), ClientError>;
    fn unpublish(&self, id: PubId);
    fn unsubscribe(&self, id: SubId);
}

/// An event queue, from [`Client::events`](crate::Client::events) (of
/// [`Event`](crate::Event)s) or [`Server::events`](crate::Server::events)
/// (of [`ServerEvent`](crate::ServerEvent)s).
#[derive(Debug)]
pub struct Events<E = crate::Event> {
    rx: mpsc::UnboundedReceiver<E>,
}

impl<E> Events<E> {
    pub(crate) fn new() -> (mpsc::UnboundedSender<E>, Self) {
        let (tx, rx) = mpsc::unbounded_channel();
        (tx, Self { rx })
    }

    /// The next event, waiting if there is none. `None` once the client or
    /// server has stopped and every event has been read.
    pub async fn recv(&mut self) -> Option<E> {
        self.rx.recv().await
    }

    /// The next event if one is waiting, without waiting.
    pub fn try_recv(&mut self) -> Option<E> {
        self.rx.try_recv().ok()
    }

    /// How many events are waiting.
    pub fn len(&self) -> usize {
        self.rx.len()
    }

    /// Whether no events are waiting.
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

/// A publisher of any type, from
/// [`Client::publish_generic`](crate::Client::publish_generic) or
/// [`Server::publish_generic`](crate::Server::publish_generic). Dropping it
/// unpublishes.
pub struct GenericPublisher {
    backend: Arc<dyn Backend>,
    id: PubId,
}

impl std::fmt::Debug for GenericPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericPublisher")
            .field("id", &self.id)
            .finish()
    }
}

impl GenericPublisher {
    pub(crate) fn new(backend: Arc<dyn Backend>, id: PubId) -> Self {
        Self { backend, id }
    }

    /// Set the value, timestamped now.
    pub fn set(&self, value: Value) -> Result<(), ClientError> {
        let now = self.backend.now_us();
        self.set_at(value, now)
    }

    /// Set the value with a timestamp from the runtime's `now_us`; 0 sets it
    /// as a default that any other value overrides.
    pub fn set_at(&self, value: Value, time_us: i64) -> Result<(), ClientError> {
        self.backend.set_value(self.id, value, time_us)
    }
}

impl Drop for GenericPublisher {
    fn drop(&mut self) {
        self.backend.unpublish(self.id);
    }
}

/// A typed publisher, from [`Client::publish`](crate::Client::publish) or
/// [`Server::publish`](crate::Server::publish). Dropping it unpublishes.
#[derive(Debug)]
pub struct Publisher<T> {
    inner: GenericPublisher,
    _ty: PhantomData<fn(T)>,
}

impl<T: NtValue> Publisher<T> {
    pub(crate) fn new(inner: GenericPublisher) -> Self {
        Self {
            inner,
            _ty: PhantomData,
        }
    }

    /// Set the value, timestamped now.
    pub fn set(&self, value: T) {
        // The type always matches, so this cannot fail.
        let _ = self.inner.set(value.into_value());
    }

    /// Set the value with a timestamp from the runtime's `now_us`.
    pub fn set_at(&self, value: T, time_us: i64) {
        let _ = self.inner.set_at(value.into_value(), time_us);
    }

    /// Set a default: sent with timestamp 0, so any other publisher's value
    /// wins over it.
    pub fn set_default(&self, value: T) {
        let _ = self.inner.set_at(value.into_value(), 0);
    }
}

/// A subscription; dropping it unsubscribes.
pub struct SubscriptionHandle {
    backend: Arc<dyn Backend>,
    id: SubId,
}

impl SubscriptionHandle {
    pub(crate) fn new(backend: Arc<dyn Backend>, id: SubId) -> Self {
        Self { backend, id }
    }
}

impl std::fmt::Debug for SubscriptionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionHandle")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        self.backend.unsubscribe(self.id);
    }
}

/// Returned by [`Subscriber::changed`] after the client or server stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStopped;

impl std::fmt::Display for ClientStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the NT client or server has stopped")
    }
}

impl std::error::Error for ClientStopped {}

/// A typed subscriber to one topic, from
/// [`Client::subscribe`](crate::Client::subscribe) or
/// [`Server::subscribe`](crate::Server::subscribe).
#[derive(Debug)]
pub struct Subscriber<T> {
    handle: SubscriptionHandle,
    rx: watch::Receiver<Option<TopicValue>>,
    _ty: PhantomData<fn() -> T>,
}

impl<T: NtValue> Subscriber<T> {
    pub(crate) fn new(handle: SubscriptionHandle, rx: watch::Receiver<Option<TopicValue>>) -> Self {
        Self {
            handle,
            rx,
            _ty: PhantomData,
        }
    }

    /// The latest value, or `None` if nothing (of type `T`) has arrived.
    /// A client keeps the last value after a disconnect.
    pub fn get(&self) -> Option<T> {
        self.rx
            .borrow()
            .as_ref()
            .and_then(|v| T::from_value(&v.value))
    }

    /// The latest value, or `default`.
    pub fn get_or(&self, default: T) -> T {
        self.get().unwrap_or(default)
    }

    /// The latest value with its timestamps.
    pub fn get_timestamped(&self) -> Option<(T, TopicValue)> {
        let v = self.rx.borrow().clone()?;
        Some((T::from_value(&v.value)?, v))
    }

    /// Wait for a value newer than the latest one when called, then return
    /// it (`Ok(None)` if it has another type than `T`). Errors once the
    /// client or server has stopped.
    pub async fn changed(&mut self) -> Result<Option<T>, ClientStopped> {
        // Only values that arrive after this call count.
        self.rx.borrow_and_update();
        self.rx.changed().await.map_err(|_| ClientStopped)?;
        Ok(self.get())
    }

    /// The handle that keeps this subscription alive.
    pub fn handle(&self) -> &SubscriptionHandle {
        &self.handle
    }
}
