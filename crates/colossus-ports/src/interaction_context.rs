//! Opaque host interaction routing carried only by explicitly scoped execution.

use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Immutable, nonserializable host routing context. This carries no effect grant.
///
/// The runtime captures it at an exact parent run and explicitly restores it for
/// that parent's child tasks. An absent context stays empty; task spawning never
/// implicitly inherits another running task's context.
#[derive(Clone, Default)]
pub struct RunInteractionContext {
    entries: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

thread_local! {
    static CURRENT: RefCell<Option<RunInteractionContext>> = const { RefCell::new(None) };
}

impl RunInteractionContext {
    /// Capture the current trusted routing context, or an empty context.
    pub fn capture() -> Self {
        CURRENT.with(|current| current.borrow().clone().unwrap_or_default())
    }

    /// Read one host's typed routing slot from the current execution.
    pub fn current<T: Any + Send + Sync>() -> Option<Arc<T>> {
        CURRENT.with(|current| {
            current
                .borrow()
                .as_ref()?
                .entries
                .get(&TypeId::of::<T>())?
                .clone()
                .downcast()
                .ok()
        })
    }

    /// Extend this context with one immutable host-owned routing slot.
    pub fn with<T: Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        Arc::make_mut(&mut self.entries).insert(TypeId::of::<T>(), value);
        self
    }

    /// Restore this exact context while polling a future, including after it moves
    /// between executor threads. Nested scopes restore the previous context.
    pub fn scope<F: Future>(self, future: F) -> impl Future<Output = F::Output> {
        ScopedInteractionFuture {
            context: self,
            future: Box::pin(future),
        }
    }
}

struct ScopedInteractionFuture<F: Future> {
    context: RunInteractionContext,
    future: Pin<Box<F>>,
}

struct RestoreInteractionContext(Option<RunInteractionContext>);

impl Drop for RestoreInteractionContext {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = self.0.take());
    }
}

impl<F: Future> Future for ScopedInteractionFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let previous = CURRENT.with(|current| current.replace(Some(this.context.clone())));
        let _restore = RestoreInteractionContext(previous);
        this.future.as_mut().poll(cx)
    }
}

#[cfg(test)]
#[path = "interaction_context_tests.rs"]
mod tests;
