use super::*;

#[tokio::test]
async fn native_signout_retires_the_original_worker() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let shared = Mutex::new(Snapshot::native(
            None,
            bridge::contract::DEFAULT_SERVICE_LANGUAGE,
        ));
        let lifetime = TitlePresenceLifetime::new();
        let (requests, receive) = bounded(1);
        requests.send(()).unwrap();
        let guard = lifetime.worker_guard();
        run(&shared, &receive, &lifetime, std::future::pending()).await;
        drop(guard);
        assert_eq!(
            lifetime.retirement(),
            Some(TitlePresenceRetirement::Complete)
        );
        assert!(lifetime.worker_finished());
    })
    .await
    .expect("native signout owner did not retire");
}

#[cfg(all(feature = "native-networking", not(target_arch = "wasm32")))]
#[tokio::test]
async fn final_process_stop_finishes_even_an_unpolled_original_product_owner() {
    let api = crate::product_api::Client::default();
    let shared = Mutex::new(Snapshot {
        native_product_api: Some(api.clone()),
        ..Snapshot::default()
    });
    let (_requests, received) = bounded(1);
    let lifetime = TitlePresenceLifetime::new();
    let (execution, stop) = bridge::tasks::NativeExecution::for_process(
        Handle::current(),
        tokio_util::task::TaskTracker::new(),
    );
    stop.stop();
    run(&shared, &received, &lifetime, execution.stopped()).await;
    let mut finished = std::pin::pin!(api.finished());
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(finished.as_mut(), &mut context).is_ready());
}
