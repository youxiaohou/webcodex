use super::super::runner_recent::RunnerRecentCollector;
use super::*;

fn sorted_projection(
    raw: &[crate::tool_runtime::sessions::WorkflowSessionConsoleListItem],
) -> Vec<RuntimeConsoleRecentSession> {
    let mut projected = raw
        .iter()
        .cloned()
        .map(|session| RuntimeConsoleRecentSession {
            client_id: "r".into(),
            project_id: "agent:r:p".into(),
            project_name: Some("Project".into()),
            session,
        })
        .collect::<Vec<_>>();
    // Existing Runner detail contract is stable updated_at descending only.
    projected.sort_by(|a, b| b.session.updated_at.cmp(&a.session.updated_at));
    projected.truncate(DEFAULT_MAX_SESSIONS);
    projected
}

fn heap_projection(
    raw: Vec<crate::tool_runtime::sessions::WorkflowSessionConsoleListItem>,
) -> RuntimeConsoleRecentSessions {
    let mut collector = RunnerRecentCollector::default();
    for session in raw {
        collector.offer("r", "agent:r:p", Some("Project"), session);
    }
    collector.finish(false)
}

#[test]
fn runner_recent_keeps_stable_timestamp_ties_and_exact_count() {
    let prototype = recent_test_row("r", "agent:r:p", "prototype", 0, false, false, false).session;
    for size in [0_usize, 1, 99, 100, 101, 377, 1000] {
        let raw = (0..size)
            .map(|index| {
                let mut session = prototype.clone();
                session.session_id = format!("session-{index:04}");
                session.updated_at = ((index * 19) % 83) as i64;
                session
            })
            .collect::<Vec<_>>();
        let sorted = sorted_projection(&raw);
        let ranked = heap_projection(raw);
        assert_eq!(ranked.candidate_count, size);
        assert_eq!(ranked.returned, size.min(DEFAULT_MAX_SESSIONS));
        assert_eq!(ranked.truncated, size > DEFAULT_MAX_SESSIONS);
        assert_eq!(
            serde_json::to_value(&ranked.sessions).unwrap(),
            serde_json::to_value(&sorted).unwrap()
        );
    }
}

#[tokio::test]
async fn runner_detail_batched_projects_preserve_session_counts_and_auth() {
    let runtime = test_runtime();
    let alice = crate::auth::shared_key_context("runner-batch-a");
    let bob = crate::auth::shared_key_context("runner-batch-b");
    register_project(&runtime, "runner-batch", "p0", "/private/p0", Some(&alice)).await;
    let project_ids = (0..7).map(|i| format!("p{i}")).collect::<Vec<_>>();
    crate::test_support::apply_project_inventory_snapshot(
        &runtime.runner_registry,
        "runner-batch",
        "inst-runner-batch",
        project_ids
            .iter()
            .map(|id| project(id, &format!("/private/{id}")))
            .collect(),
    )
    .await;
    for (index, id) in project_ids.iter().enumerate() {
        for _ in 0..(index + 1) {
            start_authorized_session(&runtime, &format!("agent:runner-batch:{id}"), &alice);
        }
    }
    let expected = (1..=7).sum::<usize>();
    let actual = runner_for_auth(&runtime, &alice, "runner-batch", None)
        .await
        .unwrap();
    assert_eq!(actual.projects_returned, 7);
    assert_eq!(actual.recent_sessions.candidate_count, expected);
    assert_eq!(actual.recent_sessions.returned, expected);
    assert!(!actual.recent_sessions.truncated);
    assert!(!actual.recent_sessions.scan_truncated);
    assert_eq!(
        actual
            .projects
            .iter()
            .map(|p| p.sessions.retained_sessions)
            .sum::<usize>(),
        expected
    );
    let outsider = runner_for_auth(&runtime, &bob, "runner-batch", None).await;
    assert!(matches!(outsider, Err(RuntimeConsoleError::NotFound)));
}

#[test]
#[ignore = "manual Runner detail projection comparison for 30,000 Session candidates"]
fn runner_recent_bounded_projection_cost_comparison() {
    use std::time::Instant;
    let prototype = recent_test_row("r", "agent:r:p", "prototype", 0, false, false, false).session;
    let fixture = (0..30_000)
        .map(|index| {
            let mut item = prototype.clone();
            item.session_id = format!("session-{index:05}");
            item.updated_at = ((index * 137) % 30_000) as i64;
            item
        })
        .collect::<Vec<_>>();
    let expected = sorted_projection(&fixture);
    let actual = heap_projection(fixture.clone());
    assert_eq!(
        serde_json::to_value(&actual.sessions).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let mut full_ns = 0u128;
    let mut heap_ns = 0u128;
    let n = 14u128;
    for iteration in 0..n {
        let baseline_input = fixture.clone();
        let optimized_input = fixture.clone();
        if iteration % 2 == 0 {
            let start = Instant::now();
            std::hint::black_box(sorted_projection(&baseline_input));
            full_ns += start.elapsed().as_nanos();
            let start = Instant::now();
            std::hint::black_box(heap_projection(optimized_input));
            heap_ns += start.elapsed().as_nanos();
        } else {
            let start = Instant::now();
            std::hint::black_box(heap_projection(optimized_input));
            heap_ns += start.elapsed().as_nanos();
            let start = Instant::now();
            std::hint::black_box(sorted_projection(&baseline_input));
            full_ns += start.elapsed().as_nanos();
        }
    }
    eprintln!("RUNNER_RECENT_PROJECTION candidates=30000 full_mean_ms={:.3} heap_mean_ms={:.3} speedup={:.2}x", full_ns as f64/n as f64/1e6, heap_ns as f64/n as f64/1e6, full_ns as f64/heap_ns as f64);
}

#[tokio::test]
#[ignore = "manual per-Project versus batched live Session Console reads"]
async fn runner_session_batch_cost_comparison() {
    use std::time::Instant;
    let runtime = test_runtime();
    let auth = crate::auth::shared_key_context("runner-batch-bench");
    register_project(&runtime, "runner-perf", "p0", "/private/p0", Some(&auth)).await;
    let project_names = (0..48).map(|i| format!("p{i}")).collect::<Vec<_>>();
    crate::test_support::apply_project_inventory_snapshot(
        &runtime.runner_registry,
        "runner-perf",
        "inst-runner-perf",
        project_names
            .iter()
            .map(|id| project(id, &format!("/private/{id}")))
            .collect(),
    )
    .await;
    let ids = project_names
        .iter()
        .map(|id| format!("agent:runner-perf:{id}"))
        .collect::<Vec<_>>();
    for id in &ids {
        for _ in 0..20 {
            start_authorized_session(&runtime, id, &auth);
        }
    }
    let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    let initial =
        runtime.workflow_sessions_console_lists(&refs, Some(CONSOLE_AGGREGATE_SESSION_LIMIT));
    assert_eq!(initial.len(), 48);
    assert_eq!(
        initial
            .values()
            .map(|row| row.sessions.len())
            .sum::<usize>(),
        960
    );
    for id in &refs {
        let standalone =
            runtime.workflow_sessions_console_list(id, Some(CONSOLE_AGGREGATE_SESSION_LIMIT));
        let batched = initial.get(*id).unwrap();
        assert_eq!(
            serde_json::to_value(&standalone).unwrap(),
            serde_json::to_value(batched).unwrap()
        );
    }
    let mut loop_ns = 0u128;
    let mut batch_ns = 0u128;
    let n = 12u128;
    for iteration in 0..n {
        if iteration % 2 == 0 {
            let start = Instant::now();
            for id in &refs {
                std::hint::black_box(
                    runtime
                        .workflow_sessions_console_list(id, Some(CONSOLE_AGGREGATE_SESSION_LIMIT)),
                );
            }
            loop_ns += start.elapsed().as_nanos();
            let start = Instant::now();
            std::hint::black_box(
                runtime
                    .workflow_sessions_console_lists(&refs, Some(CONSOLE_AGGREGATE_SESSION_LIMIT)),
            );
            batch_ns += start.elapsed().as_nanos();
        } else {
            let start = Instant::now();
            std::hint::black_box(
                runtime
                    .workflow_sessions_console_lists(&refs, Some(CONSOLE_AGGREGATE_SESSION_LIMIT)),
            );
            batch_ns += start.elapsed().as_nanos();
            let start = Instant::now();
            for id in &refs {
                std::hint::black_box(
                    runtime
                        .workflow_sessions_console_list(id, Some(CONSOLE_AGGREGATE_SESSION_LIMIT)),
                );
            }
            loop_ns += start.elapsed().as_nanos();
        }
    }
    eprintln!("RUNNER_SESSION_LIST_READS projects=48 sessions=960 one_by_one_mean_ms={:.3} batch_mean_ms={:.3} speedup={:.2}x",
        loop_ns as f64/n as f64/1e6,batch_ns as f64/n as f64/1e6,loop_ns as f64/batch_ns as f64);
}
