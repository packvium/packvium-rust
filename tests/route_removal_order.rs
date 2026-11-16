//! The route rule's unloading search on scenes where a door leaves a long chain of boxes
//! each hidden behind the next: the verdict and its message, at a size where a
//! search that re-tests every box after every removal would be cubic.

use packvium_core::{pack_json, validate_json};
use serde_json::{Value, json};

/// A staircase along a `+x` door: box `i` sits one step left of and half a box below box
/// `i + 1`, so each is hidden by exactly its successor and only the last can leave first.
fn staircase(stops: &[i64]) -> String {
    let count = stops.len();
    let items: Vec<Value> = stops
        .iter()
        .enumerate()
        .map(|(index, stop)| {
            json!({"id": format!("b{index:04}"), "quantity": 1, "stop_index": stop,
                "dimensions": {"length": "10", "width": "10", "height": "10"}})
        })
        .collect();
    let fixed: Vec<Value> = (0..count)
        .map(|index| {
            json!({"item_type": format!("b{index:04}"), "container_type": "van", "orientation": "LWH",
                "position": {"x": (10 * index).to_string(), "y": (5 * index).to_string(), "z": "0"}})
        })
        .collect();
    json!({
        "units": {"length": "mm"},
        "configuration": {"effort_budget": {"max_search_nodes": 2_000_000}, "time_limit_ms": 60_000},
        "items": items,
        "fixed_placements": fixed,
        "containers": [{"id": "van", "access_directions": ["+x"],
            "inner_dimensions": {"length": (10 * count).to_string(),
                "width": (5 * count + 5).to_string(), "height": "10"}}]
    })
    .to_string()
}

fn issues(request: &str) -> Vec<(String, String)> {
    let result = pack_json(request).expect("the scene packs");
    validate_json(request, &result)
        .expect("the documents are judgeable")
        .into_iter()
        .map(|issue| (issue.code, issue.detail))
        .collect()
}

#[test]
fn a_long_chain_unloads_when_every_box_is_due_at_one_stop() {
    assert_eq!(issues(&staircase(&[1; 120])), []);
}

#[test]
fn the_earlier_stop_hidden_behind_a_later_one_is_reported_with_its_stop() {
    let mut stops = vec![2; 40];
    stops[0] = 1;
    let found = issues(&staircase(&stops));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "unloading_order_violation");
    assert!(
        found[0]
            .1
            .contains("stop 1 cannot fully unload ['b0000#1']"),
        "{}",
        found[0].1
    );
}
