use super::*;

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(std::task::Waker::noop()))
}

#[test]
fn pending_scopes_restore_the_thread_context_and_keep_parent_snapshots_separate() {
    let mut first = Box::pin(
        RunInteractionContext::default()
            .with(Arc::new("first".to_owned()))
            .scope(std::future::poll_fn(|_| {
                assert_eq!(
                    RunInteractionContext::current::<String>()
                        .as_deref()
                        .map(String::as_str),
                    Some("first")
                );
                Poll::<()>::Pending
            })),
    );
    let mut second = Box::pin(
        RunInteractionContext::default()
            .with(Arc::new("second".to_owned()))
            .scope(async {
                assert_eq!(
                    RunInteractionContext::current::<String>()
                        .as_deref()
                        .map(String::as_str),
                    Some("second")
                );
                RunInteractionContext::capture()
            }),
    );
    assert!(poll(first.as_mut()).is_pending());
    assert!(RunInteractionContext::current::<String>().is_none());
    let Poll::Ready(captured) = poll(second.as_mut()) else {
        panic!("second completed")
    };
    assert!(RunInteractionContext::current::<String>().is_none());
    let mut child = Box::pin(captured.scope(async {
        assert_eq!(
            RunInteractionContext::current::<String>()
                .as_deref()
                .map(String::as_str),
            Some("second")
        );
        RunInteractionContext::default()
            .scope(async {
                assert!(RunInteractionContext::current::<String>().is_none());
            })
            .await;
        assert_eq!(
            RunInteractionContext::current::<String>()
                .as_deref()
                .map(String::as_str),
            Some("second")
        );
    }));
    assert!(poll(child.as_mut()).is_ready());
    assert!(poll(first.as_mut()).is_pending());
    assert!(RunInteractionContext::current::<String>().is_none());
}

#[test]
fn unwinding_restores_context_and_contexts_are_not_implicitly_shared_between_threads() {
    let context = RunInteractionContext::default().with(Arc::new("owner".to_owned()));
    let mut future = Box::pin(context.scope(async {
        std::thread::spawn(|| assert!(RunInteractionContext::current::<String>().is_none()))
            .join()
            .expect("isolated thread");
        panic!("scope panic fixture");
    }));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll(future.as_mut()))).is_err()
    );
    assert!(RunInteractionContext::current::<String>().is_none());
}

#[test]
fn a_scoped_future_keeps_its_context_when_polled_on_another_thread() {
    let mut future = Box::pin(
        RunInteractionContext::default()
            .with(Arc::new("moving".to_owned()))
            .scope(std::future::poll_fn(|_| {
                assert_eq!(
                    RunInteractionContext::current::<String>()
                        .as_deref()
                        .map(String::as_str),
                    Some("moving")
                );
                Poll::<()>::Pending
            })),
    );
    assert!(poll(future.as_mut()).is_pending());
    let mut future = std::thread::spawn(move || {
        assert!(RunInteractionContext::current::<String>().is_none());
        assert!(poll(future.as_mut()).is_pending());
        assert!(RunInteractionContext::current::<String>().is_none());
        future
    })
    .join()
    .expect("future moved between executor threads");
    assert!(poll(future.as_mut()).is_pending());
    assert!(RunInteractionContext::current::<String>().is_none());
}
