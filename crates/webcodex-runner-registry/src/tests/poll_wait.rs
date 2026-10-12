use super::*;
use std::time::{Duration, Instant};
use webcodex_core::runner_protocol::{RunnerPollRequest, ShellRunRequest};

fn poll_req(instance: &str) -> RunnerPollRequest {
    RunnerPollRequest {
        client_id: "poll-wake".to_string(),
        runner_instance_id: instance.to_string(),
    }
}

async fn registered() -> RunnerRegistry {
    let registry = RunnerRegistry::default();
    register_with_instance(&registry, "poll-wake", "instance-one").await;
    registry
}

#[tokio::test]
async fn http_poll_wait_wakes_for_real_enqueued_work_without_losing_lease_fences() {
    let registry = registered().await;
    let task_registry = registry.clone();
    let started = Instant::now();
    let task = tokio::spawn(async move {
        task_registry
            .poll_wait(poll_req("instance-one"), Duration::from_secs(4))
            .await
    });
    // Ensure this request is already waiting before enqueue.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (request_id, _result) = registry
        .enqueue_run(
            ShellRunRequest {
                login: false,
                client_id: "poll-wake".into(),
                cwd: None,
                command: "echo poll wake".into(),
                stdin: None,
                timeout_secs: 10,
                wait_timeout_secs: 0,
            },
            "tester".into(),
        )
        .await
        .unwrap();
    let request = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("event-driven poll should wake in under 1s")
        .unwrap()
        .unwrap()
        .expect("enqueued request should be delivered");
    assert_eq!(request.request_id, request_id);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        registry
            .get_runner_view("poll-wake")
            .await
            .unwrap()
            .pending_requests,
        0
    );
}

#[tokio::test]
async fn http_poll_wait_empty_timeout_and_stale_instance_are_bounded() {
    let registry = registered().await;
    let started = Instant::now();
    assert!(registry
        .poll_wait(poll_req("instance-one"), Duration::from_millis(130))
        .await
        .unwrap()
        .is_none());
    assert!(started.elapsed() >= Duration::from_millis(110));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(registry
        .poll_wait(poll_req("instance-foreign"), Duration::from_secs(4))
        .await
        .unwrap_err()
        .contains("stale or replaced"));
}

#[tokio::test]
async fn http_poll_wait_stale_after_process_takeover_may_not_dequeue_new_work() {
    let registry = registered().await;
    let old_registry = registry.clone();
    let old = tokio::spawn(async move {
        old_registry
            .poll_wait(poll_req("instance-one"), Duration::from_millis(450))
            .await
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    register_with_instance(&registry, "poll-wake", "instance-two").await;
    let error = old.await.unwrap().unwrap_err();
    assert!(error.contains("stale or replaced"), "{error}");
}

/// Exact transport-level idle progression comparison: the current legacy
/// Runner waits 5s after an empty successful poll. The new Runner instead
/// waits inside the Server and returns as soon as a request is queued.
/// Includes actual registry dequeue, but excludes network/Runner process time.
#[tokio::test]
#[ignore = "manual 5-second legacy idle-poll vs event wake benchmark"]
async fn http_poll_wait_seconds_level_idle_dispatch_comparison() {
    let older = registered().await;
    let old_started = Instant::now();
    assert!(older
        .poll(poll_req("instance-one"))
        .await
        .unwrap()
        .is_none());
    let old_registry = older.clone();
    let old_sender = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        old_registry
            .enqueue_run(
                ShellRunRequest {
                    login: false,
                    client_id: "poll-wake".into(),
                    cwd: None,
                    command: "echo old".into(),
                    stdin: None,
                    timeout_secs: 10,
                    wait_timeout_secs: 0,
                },
                "tester".into(),
            )
            .await
            .unwrap();
    });
    // Exact existing idle backoff: no server notification in HTTP polling.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(older
        .poll(poll_req("instance-one"))
        .await
        .unwrap()
        .is_some());
    old_sender.await.unwrap();
    let old_secs = old_started.elapsed().as_secs_f64();

    let faster = registered().await;
    let fast_registry = faster.clone();
    let fast_started = Instant::now();
    let fast_sender = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        fast_registry
            .enqueue_run(
                ShellRunRequest {
                    login: false,
                    client_id: "poll-wake".into(),
                    cwd: None,
                    command: "echo new".into(),
                    stdin: None,
                    timeout_secs: 10,
                    wait_timeout_secs: 0,
                },
                "tester".into(),
            )
            .await
            .unwrap();
    });
    let outcome = faster
        .poll_wait(poll_req("instance-one"), Duration::from_millis(3500))
        .await
        .unwrap();
    assert!(outcome.is_some());
    fast_sender.await.unwrap();
    let fast_secs = fast_started.elapsed().as_secs_f64();
    eprintln!("POLLING_WAKE_SECONDS legacy_idle_dispatch_s={old_secs:.3} notified_idle_dispatch_s={fast_secs:.3} saved_s={:.3}", old_secs-fast_secs);
    assert!(old_secs >= 5.0);
    assert!(fast_secs < 1.0);
}
