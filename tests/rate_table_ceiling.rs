//! M1, the money ceiling at admission: a rate table whose dearest landed cost -- top price or
//! minimum charge, plus the fuel surcharge on it -- passes 2^53 - 1 is refused, whatever the
//! objective and by whichever door the request came in: `pack_json` or `rebalance_json`.
//! The typed `pack_request` door is pinned in `api::currency_tests`, since only the crate can
//! build a rate table.

use packvium_core::{PackError, pack_json, rebalance_json};
use serde_json::{Value, json};

const REASON: &str = "above_maximum";
const POINTER: &str = "/containers/1/rate_table";
const MESSAGE: &str =
    "must price its top bracket at most 9007199254740991 minor units with the fuel surcharge";
const CEILING: i64 = 9_007_199_254_740_991;

fn json_request(table: Value, objective: &str) -> Value {
    let mut request = json!({
        "items": [{ "id": "cube", "quantity": 2, "weight": "1000",
                    "dimensions": { "length": "100", "width": "100", "height": "100" } }],
        "containers": [
            { "id": "plain",
              "inner_dimensions": { "length": "200", "width": "100", "height": "200" } },
            { "id": "box",
              "inner_dimensions": { "length": "200", "width": "100", "height": "200" },
              "rate_table": table },
        ],
        "configuration": { "solver_profile": "fast", "objective": objective,
                           "dimensional_weight_divisor": 5000 }
    });
    // The landed-cost objective prices every container.
    if objective == "lowest_landed_cost" {
        request["containers"][0]["rate_table"] =
            json!({ "weight_brackets_g": [5000], "prices_minor": [1] });
    }
    request
}

fn table(prices: &[i64], minimum: i64, fuel: i64) -> Value {
    json!({ "weight_brackets_g": (1..=prices.len()).map(|n| n * 5000).collect::<Vec<_>>(),
            "prices_minor": prices, "minimum_charge_minor": minimum,
            "fuel_surcharge_permille": fuel })
}

fn assert_refused(error: PackError) {
    match error {
        PackError::InvalidRequest(error) => assert_eq!(
            (error.reason(), error.field(), error.detail()),
            (REASON, POINTER, MESSAGE)
        ),
        other => panic!("expected the M1 refusal, got {other}"),
    }
}

#[test]
fn a_landed_cost_exactly_at_the_ceiling_is_admitted() {
    // 2 * 4503599627370495 = 2^53 - 2; the minimum charge at the ceiling needs no surcharge.
    for objective in ["default", "lowest_landed_cost"] {
        for table in [
            table(&[CEILING], 0, 0),
            table(&[1, 4_503_599_627_370_495], 0, 1000),
            table(&[1], CEILING, 0),
            table(&[CEILING], 0, 0),
        ] {
            pack_json(&json_request(table, objective).to_string()).expect("inside the ceiling");
        }
    }
}

#[test]
fn a_landed_cost_one_past_the_ceiling_is_refused_on_the_json_path() {
    for objective in ["default", "lowest_landed_cost"] {
        for table in [
            table(&[4_503_599_627_370_496], 0, 1000),
            table(&[1], CEILING, 1),
            table(&[CEILING - 1, CEILING], 0, CEILING),
            // The surcharge is rounded up: 9e15 + ceil(9e15 / 1000) = 9.009e15.
            table(&[9_000_000_000_000_000], 0, 1),
        ] {
            let error = pack_json(&json_request(table, objective).to_string())
                .expect_err("past the ceiling");
            assert_refused(error);
        }
    }
}

#[test]
fn a_table_without_prices_is_left_to_the_existing_rules() {
    let mut request = json_request(table(&[], CEILING, CEILING), "default");
    request["containers"][1]["rate_table"]["weight_brackets_g"] = json!([]);
    if let Err(PackError::InvalidRequest(error)) = pack_json(&request.to_string()) {
        assert_ne!(error.detail(), MESSAGE);
    }
}

#[test]
fn the_rebalance_entry_refuses_it_too() {
    let admitted = json_request(table(&[CEILING], 0, 0), "default");
    let result = pack_json(&admitted.to_string()).expect("packs");
    let refused = json_request(table(&[CEILING], 0, 1), "default");
    assert_refused(rebalance_json(&refused.to_string(), &result, 4).expect_err("past the ceiling"));
}

#[test]
fn the_overflowing_landed_cost_is_not_reported_as_a_bracket_miss() {
    // C-20: this table once surfaced as "bills at 600 g, above its rate table's last bracket".
    let error = pack_json(
        &json_request(
            table(&[CEILING - 1, CEILING], 0, CEILING),
            "lowest_landed_cost",
        )
        .to_string(),
    )
    .expect_err("refused");
    assert!(!error.to_string().contains("last bracket"), "{error}");
}
