//! Historical Job-scan avoidance and active-count parity for Runtime overview.
use super::*;

async fn register_console_job_count_fixture(
    runtime: &ToolRuntime,
    auth: &AuthContext,
    count: usize,
) {
    register_project(
        runtime,
        "job-overview",
        "repo",
        "/private/job-overview",
        Some(auth),
    )
    .await;
    let access = crate::test_support::runner_access(auth);
    for index in 0..count {
        runtime
            .runner_registry
            .start_job_with_metadata_for_access(
                crate::runner_protocol::ShellJobOpRequest {
                    login: false,
                    op: "start".to_string(),
                    client_id: Some("job-overview".to_string()),
                    cwd: None,
                    command: Some(format!("echo read-only-overview-{index}")),
                    timeout_secs: Some(60),
                    job_id: None,
                    since_stdout_line: None,
                    since_stderr_line: None,
                    tail_lines: None,
                    limit: None,
                    codex: None,
                },
                "overview-fixture".into(),
                webcodex_runner_registry::ShellJobStartMetadata {
                    project_id: Some("agent:job-overview:repo".to_string()),
                    ..Default::default()
                },
                Some(&access),
                None,
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn full_overview_reuses_authorized_active_jobs_without_historical_job_scan() {
    let runtime = test_runtime();
    let alice = crate::auth::shared_key_context("overview-active-a");
    let bob = crate::auth::shared_key_context("overview-active-b");
    register_console_job_count_fixture(&runtime, &alice, 7).await;
    let previous = runtime_status_value(&runtime, &alice, None).await.unwrap();
    assert_eq!(previous["jobs"]["active_count"], 7);
    let history_scans = runtime
        .runner_registry
        .full_job_history_scan_count_for_test();

    let home = overview_for_auth_detail(&runtime, &alice, false)
        .await
        .unwrap();
    assert_eq!(home.active_jobs, 7);
    assert_eq!(home.runners.len(), 1);
    assert_eq!(home.runners[0].active_jobs, 7);
    assert_eq!(
        runtime
            .runner_registry
            .full_job_history_scan_count_for_test(),
        history_scans,
        "full-project overview should use active summaries without another historical scan"
    );
    let outsider = overview_for_auth_detail(&runtime, &bob, false)
        .await
        .unwrap();
    assert_eq!(outsider.active_jobs, 0);
    assert_eq!(outsider.runner_count, 0);
    assert_eq!(
        runtime
            .runner_registry
            .full_job_history_scan_count_for_test(),
        history_scans
    );
}

#[tokio::test]
#[ignore = "manual Runtime Console fetch comparison with 200 real queued Runner Jobs"]
async fn full_overview_duplicate_status_fetch_cost_comparison() {
    use std::time::Instant;
    let runtime = test_runtime();
    let auth = crate::auth::shared_key_context("overview-fetch-bench");
    register_console_job_count_fixture(&runtime, &auth, 200).await;
    let baseline = runtime_status_value(&runtime, &auth, None).await.unwrap();
    assert_eq!(baseline["jobs"]["active_count"], 200);
    let home = overview_for_auth_detail(&runtime, &auth, false)
        .await
        .unwrap();
    assert_eq!(home.active_jobs, 200);
    let mut old_ns = 0_u128;
    let mut new_ns = 0_u128;
    let iterations = 12_u128;
    for index in 0..iterations {
        if index % 2 == 0 {
            let started = Instant::now();
            let (status, runners) = tokio::try_join!(
                runtime_status_value(&runtime, &auth, None),
                list_runners_value(&runtime, &auth, None),
            )
            .unwrap();
            assert_eq!(
                status["jobs"]["active_count"],
                runners["summary"]["active_jobs_total"]
            );
            std::hint::black_box((status, runners));
            old_ns += started.elapsed().as_nanos();
        }
        let started = Instant::now();
        let runners = list_runners_value(&runtime, &auth, None).await.unwrap();
        assert_eq!(runners["summary"]["active_jobs_total"], 200);
        std::hint::black_box(runners);
        new_ns += started.elapsed().as_nanos();
        if index % 2 != 0 {
            let started = Instant::now();
            let (status, runners) = tokio::try_join!(
                runtime_status_value(&runtime, &auth, None),
                list_runners_value(&runtime, &auth, None),
            )
            .unwrap();
            assert_eq!(
                status["jobs"]["active_count"],
                runners["summary"]["active_jobs_total"]
            );
            std::hint::black_box((status, runners));
            old_ns += started.elapsed().as_nanos();
        }
    }
    println!(
        "RUNTIME_OVERVIEW_STATUS_FETCH jobs=200 duplicate_fetch_ms={:.3} single_fetch_ms={:.3} speedup={:.2}x",
        old_ns as f64 / iterations as f64 / 1e6,
        new_ns as f64 / iterations as f64 / 1e6,
        old_ns as f64 / new_ns as f64
    );
}
