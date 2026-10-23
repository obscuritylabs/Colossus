use super::*;

#[tokio::test]
async fn completed_worker_is_joined_only_once() {
    let worker = Mutex::new(Some(tokio::spawn(async { Ok(()) })));
    join_stopped_worker(&worker, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(worker.lock().await.is_none());
    join_stopped_worker(&worker, Duration::from_secs(1))
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_worker_is_reported_and_its_completed_handle_is_removed() {
    let worker = Mutex::new(Some(tokio::spawn(async { Err("worker failed") })));
    assert!(matches!(
        join_stopped_worker(&worker, Duration::from_secs(1)).await,
        Err(CloudError::Storage)
    ));
    assert!(worker.lock().await.is_none());
    join_stopped_worker(&worker, Duration::from_secs(1))
        .await
        .unwrap();
}

#[tokio::test]
async fn cancelled_worker_is_reported_and_its_completed_handle_is_removed() {
    let task = tokio::spawn(std::future::pending::<std::result::Result<(), &'static str>>());
    task.abort();
    let worker = Mutex::new(Some(task));
    assert!(matches!(
        join_stopped_worker(&worker, Duration::from_secs(1)).await,
        Err(CloudError::Storage)
    ));
    assert!(worker.lock().await.is_none());
    join_stopped_worker(&worker, Duration::from_secs(1))
        .await
        .unwrap();
}

#[tokio::test]
async fn timed_out_join_retains_the_worker_until_it_finishes() {
    let (finish, finished) = tokio::sync::oneshot::channel();
    let worker = Mutex::new(Some(tokio::spawn(async move {
        finished.await.unwrap();
        Ok(())
    })));
    assert!(matches!(
        join_stopped_worker(&worker, Duration::ZERO).await,
        Err(CloudError::Storage)
    ));
    assert!(
        worker
            .lock()
            .await
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    );
    finish.send(()).unwrap();
    join_stopped_worker(&worker, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(worker.lock().await.is_none());
}

#[tokio::test]
async fn cancelled_join_retains_the_worker_for_later_retirement() {
    let (finish, finished) = tokio::sync::oneshot::channel();
    let worker = Mutex::new(Some(tokio::spawn(async move {
        finished.await.unwrap();
        Ok(())
    })));
    {
        let joining = join_stopped_worker(&worker, Duration::from_secs(1));
        tokio::pin!(joining);
        tokio::select! {
            biased;
            result = &mut joining => panic!("pending worker unexpectedly joined: {result:?}"),
            () = tokio::task::yield_now() => {},
        }
    }
    assert!(
        worker
            .lock()
            .await
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    );
    finish.send(()).unwrap();
    join_stopped_worker(&worker, Duration::from_secs(1))
        .await
        .unwrap();
    assert!(worker.lock().await.is_none());
}
