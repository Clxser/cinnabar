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
        run(&shared, &receive, &lifetime).await;
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
