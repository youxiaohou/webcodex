//! Owned nested argument admission and move semantics.

use super::*;
use serde_json::json;

// Frozen former implementation, including exact rejection precedence and
// unaltered Project/Session injection, for independent output comparison.
fn legacy_prepare_arguments(
    arguments: Value,
    project: &str,
    session_id: &str,
    additional_forbidden_fields: &[&str],
) -> Result<Value, OrchestrationHostError> {
    let Some(mut arguments) = arguments.as_object().cloned() else {
        return Err(OrchestrationHostError::new(
            OrchestrationHostFailureKind::InvalidArguments,
            "nested tool arguments must be a JSON object",
        ));
    };
    if let Some(field) = arguments
        .keys()
        .find(|field| is_server_owned_orchestration_argument(field))
    {
        return Err(OrchestrationHostError::new(
            OrchestrationHostFailureKind::InvalidArguments,
            format!("nested tool arguments may not set server-owned field `{field}`"),
        ));
    }
    if let Some(field) = additional_forbidden_fields
        .iter()
        .find(|field| arguments.contains_key(**field))
    {
        return Err(OrchestrationHostError::new(
            OrchestrationHostFailureKind::InvalidArguments,
            format!("nested tool arguments may not set frontend-reserved field `{field}`"),
        ));
    }
    arguments.insert("project".to_string(), Value::String(project.to_string()));
    arguments.insert(
        "session_id".to_string(),
        Value::String(session_id.to_string()),
    );
    Ok(Value::Object(arguments))
}

#[test]
fn owned_argument_preparation_matches_frozen_rejection_and_scope_injection() {
    let cases = [
        Value::Null,
        json!(42),
        json!(true),
        json!("not an object"),
        json!([]),
        json!({}),
        json!({"items": [{"path": "src/lib.rs", "limit": 120}]}),
        json!({"source": {"nested": ["界", "\n", "\t", 42], "closed": false}}),
        json!({
            "script": "👾\n\t\"".repeat(16_000),
            "items": (0..32).map(|index| json!({
                "index": index,
                "action": "observe",
                "metadata": {"nested": ["unicode 界", "escape \\"]}
            })).collect::<Vec<_>>()
        }),
    ];
    for (index, value) in cases.into_iter().enumerate() {
        for reserved in [&[][..], &["frontend_only", "receipt"][..]] {
            let expected = legacy_prepare_arguments(
                value.clone(),
                "agent:runner:resolved",
                "wc_sess_canonical",
                reserved,
            );
            let actual = prepare_orchestration_arguments(
                value.clone(),
                "agent:runner:resolved",
                "wc_sess_canonical",
                reserved,
            );
            assert_eq!(actual, expected, "case={index} reserved={reserved:?}");
            if let Ok(output) = actual {
                assert_eq!(output["project"], "agent:runner:resolved");
                assert_eq!(output["session_id"], "wc_sess_canonical");
                assert_eq!(
                    output.as_object().unwrap().len(),
                    value.as_object().unwrap().len() + 2
                );
            }
        }
    }
}

#[test]
fn forbidden_argument_fields_keep_exact_error_precedence_and_fence() {
    for field in SERVER_OWNED_ARGUMENT_FIELDS
        .iter()
        .copied()
        .chain(["__webcodex_private", "__webcodex_new_future_field"])
    {
        let mut arguments = json!({"nested": {"payload": 1}});
        arguments[field] = json!({"unexpected": "retarget"});
        let expected = legacy_prepare_arguments(
            arguments.clone(),
            "agent:fixed",
            "wc_sess_fixed",
            &["frontend_only"],
        );
        let actual = prepare_orchestration_arguments(
            arguments,
            "agent:fixed",
            "wc_sess_fixed",
            &["frontend_only"],
        );
        assert_eq!(actual, expected, "field={field}");
        let error = actual.unwrap_err();
        assert_eq!(
            error.failure_kind(),
            OrchestrationHostFailureKind::InvalidArguments,
        );
        assert!(error.into_message().contains("server-owned field"));
    }

    let mut reserved = json!({"frontend_only": 7});
    for extra in [false, true] {
        if extra {
            reserved["project"] = json!("wrong-project");
        }
        let before = legacy_prepare_arguments(
            reserved.clone(),
            "agent:fixed",
            "wc_sess_fixed",
            &["frontend_only"],
        );
        let after = prepare_orchestration_arguments(
            reserved.clone(),
            "agent:fixed",
            "wc_sess_fixed",
            &["frontend_only"],
        );
        assert_eq!(after, before);
        assert_eq!(
            after.unwrap_err().failure_kind(),
            OrchestrationHostFailureKind::InvalidArguments
        );
    }
}

#[test]
fn owned_argument_payload_is_moved_not_deep_cloned() {
    let text = "large text and unicode 界🦀".repeat(15_000);
    let original = json!({
        "script": text,
        "nested": {"items": ["one", "two", {"payload": "nested-data"}]}
    });
    let original_ptr = original["script"].as_str().unwrap().as_ptr();
    let actual =
        prepare_orchestration_arguments(original, "agent:resolved", "wc_sess_resolved", &[])
            .expect("ordinary nested inputs remain allowed");
    assert_eq!(
        actual["script"].as_str().unwrap().as_ptr(),
        original_ptr,
        "owned JSON payload must retain its existing buffer"
    );
    assert_eq!(actual["project"], "agent:resolved");
    assert_eq!(actual["session_id"], "wc_sess_resolved");
}
