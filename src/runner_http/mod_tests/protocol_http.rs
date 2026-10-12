use super::*;

#[tokio::test]
async fn polling_http_register_requires_generation() {
    use salvo::test::{ResponseExt, TestClient};
    use salvo::Service;

    let registry = Arc::new(RunnerRegistry::default());
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(registry.clone()))
            .hoop(affix_state::inject(auth_context(None, true)))
            .push(Router::with_path("api/shell/agent/register").post(runner_register)),
    );
    let mut response = TestClient::post("http://localhost/api/shell/agent/register")
        .json(&json!({
            "client_id": "polling-missing-generation",
            "agent_instance_id": "inst",
            "capabilities": {"shell": true}
        }))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::BAD_REQUEST
    );
    let body: serde_json::Value = response.take_json().await.unwrap();
    assert_eq!(body["success"], false);
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|error| error.contains("agent_protocol_generation")),
        "{body:?}"
    );
    assert!(registry.list_runners().await.is_empty());
}

#[tokio::test]
async fn polling_http_register_accepts_generation_two() {
    use salvo::test::{ResponseExt, TestClient};
    use salvo::Service;

    let registry = Arc::new(RunnerRegistry::default());
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(registry.clone()))
            .hoop(affix_state::inject(auth_context(None, true)))
            .push(Router::with_path("api/shell/agent/register").post(runner_register)),
    );
    let mut response = TestClient::post("http://localhost/api/shell/agent/register")
        .json(&json!({
            "client_id": "polling-generation-two",
            "agent_instance_id": "inst",
            "agent_protocol_generation": RUNNER_PROTOCOL_GENERATION_V2.get(),
            "capabilities": crate::test_support::current_runner_capabilities(RunnerCapabilities::default())
        }))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK
    );
    let body: serde_json::Value = response.take_json().await.unwrap();
    assert_eq!(body["success"], true);
    let view = registry
        .get_runner_view("polling-generation-two")
        .await
        .unwrap();
    assert_eq!(
        view.runner_protocol_generation,
        RUNNER_PROTOCOL_GENERATION_V2
    );
    assert_eq!(view.transport, TRANSPORT_POLLING);
    assert!(view.projects.is_empty());
    assert_eq!(
        view.project_inventory
            .as_ref()
            .map(|status| status.sync_state.as_str()),
        Some("pending")
    );
}

#[tokio::test]
async fn polling_http_result_accepts_runner_envelope_above_global_text_limit() {
    use salvo::test::{ResponseExt, TestClient};
    use salvo::Service;

    let registry = Arc::new(RunnerRegistry::default());
    registry
        .register(runner_registration(
            "polling-large-result",
            "inst",
            Vec::new(),
        ))
        .await
        .unwrap();
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(registry))
            .hoop(affix_state::inject(auth_context(None, true)))
            .push(Router::with_path("api/shell/agent/result").post(runner_result)),
    );
    let payload = serde_json::to_string(&json!({
        "client_id": "polling-large-result",
        "agent_instance_id": "inst",
        "request_id": "missing-large-result",
        "exit_code": 0,
        "stdout": "x".repeat(3 * 1024 * 1024),
        "stderr": null,
        "duration_ms": 1,
        "error": null
    }))
    .unwrap();
    assert!(payload.len() > 2 * 1024 * 1024);
    assert!(payload.len() < RUNNER_ENVELOPE_MAX_BYTES);

    let mut response = TestClient::post("http://localhost/api/shell/agent/result")
        .add_header("content-type", "application/json", true)
        .body(payload)
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::BAD_REQUEST
    );
    let body: serde_json::Value = response.take_json().await.unwrap();
    assert_eq!(body["success"], false);
    assert!(
        !body["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Invalid JSON:"),
        "{body:?}"
    );
}

#[tokio::test]
async fn polling_http_offline_releases_only_the_matching_active_instance() {
    use salvo::test::{ResponseExt, TestClient};
    use salvo::Service;

    let registry = Arc::new(RunnerRegistry::default());
    registry
        .register(runner_registration("polling-offline", "inst-a", Vec::new()))
        .await
        .unwrap();
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(registry.clone()))
            .hoop(affix_state::inject(auth_context(None, true)))
            .push(Router::with_path("api/shell/agent/offline").post(runner_offline)),
    );

    let mut response = TestClient::post("http://localhost/api/shell/agent/offline")
        .json(&json!({
            "client_id": "polling-offline",
            "agent_instance_id": "inst-a"
        }))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK
    );
    assert_eq!(
        response.take_json::<serde_json::Value>().await.unwrap()["success"],
        true
    );
    assert!(
        !registry
            .get_runner_view("polling-offline")
            .await
            .unwrap()
            .connected
    );

    let replacement = registry
        .register(runner_registration("polling-offline", "inst-b", Vec::new()))
        .await
        .unwrap();
    assert!(replacement.connected);
    let mut stale = TestClient::post("http://localhost/api/shell/agent/offline")
        .json(&json!({
            "client_id": "polling-offline",
            "agent_instance_id": "inst-a"
        }))
        .send(&service)
        .await;
    assert_eq!(stale.status_code.unwrap_or(StatusCode::OK), StatusCode::OK);
    assert_eq!(
        stale.take_json::<serde_json::Value>().await.unwrap()["success"],
        true
    );
    let current = registry.get_runner_view("polling-offline").await.unwrap();
    assert_eq!(current.runner_instance_id, "inst-b");
    assert!(current.connected);
}

#[tokio::test]
async fn polling_http_wait_header_wakes_for_enqueued_work_and_old_protocol_stays_immediate() {
    use salvo::test::{ResponseExt, TestClient};
    use salvo::Service;
    use std::time::{Duration, Instant};

    let registry = Arc::new(RunnerRegistry::default());
    registry
        .register(runner_registration("http-push", "inst", Vec::new()))
        .await
        .unwrap();
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(registry.clone()))
            .hoop(affix_state::inject(auth_context(None, true)))
            .push(Router::with_path("api/shell/agent/poll").post(runner_poll)),
    );

    // Legacy clients omit the header: preserve the immediate, empty response
    // and omit the new opt-in acknowledgement from JSON.
    let mut legacy = TestClient::post("http://localhost/api/shell/agent/poll")
        .json(&json!({"client_id":"http-push","agent_instance_id":"inst"}))
        .send(&service)
        .await;
    let old_body: serde_json::Value = legacy.take_json().await.unwrap();
    assert_eq!(old_body["success"], true);
    assert!(old_body["request"].is_null());
    assert!(old_body.get("poll_wait_supported").is_none());

    let start = Instant::now();
    let wait = TestClient::post("http://localhost/api/shell/agent/poll")
        .add_header("x-webcodex-poll-wait-ms", "3500", true)
        .json(&json!({"client_id":"http-push","agent_instance_id":"inst"}))
        .send(&service);
    let enqueue = async {
        tokio::time::sleep(Duration::from_millis(180)).await;
        registry
            .enqueue_run(
                ShellRunRequest {
                    login: false,
                    client_id: "http-push".into(),
                    cwd: None,
                    command: "echo wake".into(),
                    stdin: None,
                    timeout_secs: 10,
                    wait_timeout_secs: 0,
                },
                "tester".into(),
            )
            .await
            .unwrap()
            .0
    };
    let (mut result, request_id) = tokio::join!(wait, enqueue);
    assert_eq!(result.status_code, Some(StatusCode::OK));
    let body: serde_json::Value = result.take_json().await.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["poll_wait_supported"], true);
    assert_eq!(body["request"]["request_id"], request_id);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "poll dispatch should beat idle 5s backoff"
    );
}
