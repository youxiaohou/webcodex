//! Borrowed search results must preserve canonical compound plans and evidence.

use super::*;

// Frozen clone-based planning behavior for independent output comparison.
fn legacy_sanitize_batch_search_and_collect_successes(batch: &mut Value) -> Vec<Value> {
    let mut successful = Vec::new();
    let Some(items) = batch.get_mut("items").and_then(Value::as_array_mut) else {
        return successful;
    };
    for item in items {
        if item.get("success").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(output) = item.get_mut("output") else {
            continue;
        };
        if let Some(matches) = output.get_mut("matches").and_then(Value::as_array_mut) {
            for matched in matches {
                if let Some(object) = matched.as_object_mut() {
                    object.remove("read_hint");
                }
            }
        }
        successful.push(output.clone());
    }
    successful
}

#[test]
fn borrowed_search_outputs_match_previous_sanitization_and_fair_read_plan() {
    let inputs = [
        json!({"items": [
            {"index": 0, "success": true, "output": {"matches": [
                {"path": "src/汉字.rs", "line": 8, "preview": "界🦀\\n\\t",
                    "context_before": ["source context"],
                    "read_hint": {"path": "src/汉字.rs", "limit": 100}},
                {"path": "src/汉字.rs", "line": 20, "preview": "tail",
                    "read_hint": {"path": "src/汉字.rs"}}
            ]}, "error": null},
            {"index": 1, "success": false, "output": {
                "reason_code": "invalid_pattern", "source": "private context"
            }, "error": "invalid pattern"},
            {"index": 2, "success": true, "output": {"matches": [
                {"path": "src/b.rs", "line": 100, "preview": "no clone",
                    "read_hint": {"path": "src/b.rs", "start_line": 1}},
                {"path": "src/b.rs", "line": 109, "preview": ""}
            ]}, "error": null}
        ], "output_truncated": false, "unknown": {"marker": 7}}),
        json!({"items": [
            {"index": 0, "success": true, "output": {"matches": "malformed"}},
            {"index": 1, "success": true, "output": null},
            {"index": 2, "success": true, "unknown": true},
            {"index": 3, "success": false, "output": null}
        ]}),
        json!({"items": [], "requested_count": 0}),
        json!({"items": "not-array"}),
        json!({}),
    ];
    for (case, original) in inputs.into_iter().enumerate() {
        let mut previous = original.clone();
        let mut candidate = original;
        let old = legacy_sanitize_batch_search_and_collect_successes(&mut previous);
        let borrowed = sanitize_batch_search_and_collect_successes(&mut candidate);
        assert_eq!(
            borrowed
                .iter()
                .map(|value| (*value).clone())
                .collect::<Vec<_>>(),
            old,
            "case={case}: successful search values/ordering changed"
        );
        for before in [0, 20, 100] {
            let planned = fair_match_read_items(borrowed.iter().copied(), before, 40, 8);
            let expected = fair_match_read_items(old.iter(), before, 40, 8);
            assert_eq!(planned.len(), expected.len(), "case={case} before={before}");
            assert_eq!(
                planned
                    .iter()
                    .map(|value| json!({
                        "path": value.path, "start_line": value.start_line, "limit": value.limit,
                    }))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|value| json!({
                        "path": value.path, "start_line": value.start_line, "limit": value.limit,
                    }))
                    .collect::<Vec<_>>(),
                "case={case} before={before}: read plan must remain exactly ordered"
            );
        }
        // End the borrowed planning snapshot before inspecting the batch
        // again; the search payload is still owned by the canonical batch.
        drop(borrowed);
        assert_eq!(
            candidate, previous,
            "case={case}: source batch mutated differently"
        );
    }
}

#[test]
fn borrowed_match_previews_and_single_query_output_retain_original_buffer() {
    let large_preview = "source preview 中文🦀\\n".repeat(8_000);
    let mut batch = json!({"items": [
        {"index": 0, "success": true, "error": null, "output": {"matches": [{
            "path": "src/a.rs", "line": 123, "preview": large_preview,
            "read_hint": {"path": "src/a.rs"}
        }]}}
    ]});
    let ptr = batch["items"][0]["output"]["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .as_ptr();
    let outputs = sanitize_batch_search_and_collect_successes(&mut batch);
    assert_eq!(outputs.len(), 1);
    assert_eq!(
        outputs[0]["matches"][0]["preview"]
            .as_str()
            .unwrap()
            .as_ptr(),
        ptr,
        "planning must borrow the exact original match buffer"
    );
    let mut projected_search = compact_compound_search(batch, &[true]);
    let previous = projected_search["items"][0]["output"].clone();
    let projected_ptr = projected_search["items"][0]["output"]["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .as_ptr();
    let moved = projected_search
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .and_then(|items| items.first_mut())
        .and_then(|item| item.get_mut("output"))
        .map(std::mem::take)
        .unwrap_or(Value::Null);
    assert_eq!(
        moved, previous,
        "single-query search result must remain byte-equivalent"
    );
    assert_eq!(
        moved["matches"][0]["preview"].as_str().unwrap().as_ptr(),
        projected_ptr,
        "single query must move output rather than copy large preview bytes"
    );
}
