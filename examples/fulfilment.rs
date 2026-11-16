//! From a paid order to labelled parcels: eligibility, packing, a second opinion, a price.
//!
//!     cargo run --example fulfilment
//!
//! The checkout example maps a cart onto a request and reads the parcels back. A warehouse
//! needs more around that call, and this example wires the pieces in the order a fulfilment
//! job runs them:
//!
//! 1. `commerce::evaluate_policy_json` decides which carrier service the order may use.
//! 2. `pack_json` packs it, under the rules the goods come with.
//! 3. `validate_json` checks the plan against the request with code that shares nothing
//!    with the solvers, so a parcel is never labelled on the solver's word alone.
//! 4. `commerce::quote_json` prices every parcel from the tariff version you pinned.
//!
//! Two kinds of trouble are kept apart on purpose. A request that is wrong (a quantity of
//! zero from a bad import) is `PackError::InvalidRequest`, and the job puts the order on
//! hold. An item that fits no carton is not an error: the rest of the order ships and the
//! item is handed to freight.

use std::collections::BTreeMap;
use std::error::Error;

use packvium_core::commerce::{evaluate_policy_json, quote_json};
use packvium_core::{PackError, pack_json, validate_json};
use serde_json::{Value, json};

/// Results carry exact integers: 16 000 length ticks to the millimetre, 8 000 000 weight
/// ticks to the gram. A carrier bills whole millimetres and grams, rounded up.
const TICKS_PER_MM: u64 = 16_000;
const TICKS_PER_GRAM: u64 = 8_000_000;

struct Order {
    id: &'static str,
    zone: &'static str,
    service: &'static str,
    lines: &'static [(&'static str, u32)],
}

/// What you publish: one tariff per carrier service, and the rules that say what may ship
/// how. You would load this from your own storage; a version's number is its position in
/// the list, so `tariff_version: 1` below always means the same prices.
fn commerce_document() -> Value {
    json!({
        "tariffs": [
            {
                "carrier_id": "postway", "service_id": "air",
                "versions": [{
                    "effective_at": 0,
                    "dimensional_weight_divisor": 5000,
                    "cost_per_dimensional_kg_minor": {"domestic": 520, "eu": 890},
                    "minimum_charge_minor": 950,
                    "fuel_surcharge_permille": 140,
                    "accessorials": []
                }]
            },
            {
                "carrier_id": "postway", "service_id": "ground",
                "versions": [{
                    "effective_at": 0,
                    "dimensional_weight_divisor": 6000,
                    "cost_per_dimensional_kg_minor": {"domestic": 210, "eu": 380},
                    "minimum_charge_minor": 590,
                    "fuel_surcharge_permille": 90,
                    "accessorials": []
                }]
            }
        ],
        "policy_rules": [{
            "rule_id": "lithium-stays-on-the-ground",
            "versions": [{
                "scope": "service", "action": "reject", "priority": 10, "effective_at": 0,
                "reason": "lithium batteries do not fly with this carrier",
                "predicates": [
                    {"scope": "service", "field": "service", "operator": "equals", "value": "air"},
                    {"scope": "service", "field": "battery", "operator": "equals", "value": "lithium"}
                ]
            }]
        }],
        "catalogs": []
    })
}

/// Your product table, already in request form: the rules a product has are the fields it
/// carries, and an absent field means "no rule". `battery` is yours, not the engine's.
fn products() -> Value {
    json!({
        "laptop": {"mm": [360, 250, 30], "g": 2100, "battery": "lithium"},
        "monitor": {"mm": [680, 430, 140], "g": 5400, "rules": {"keep_upright": true, "stackable": false}},
        "bleach": {"mm": [90, 90, 280], "g": 1100,
                   "rules": {"keep_upright": true, "tags": ["chemical"], "incompatible_tags": ["food"]}},
        "coffee": {"mm": [120, 80, 220], "g": 1000, "rules": {"tags": ["food"]}},
        "desk": {"mm": [1600, 800, 60], "g": 28000}
    })
}

fn cartons() -> Value {
    json!([
        {"id": "S", "mm": [300, 200, 300], "tare_g": 180, "max_kg": 8, "cost_minor": 50},
        {"id": "M", "mm": [400, 300, 300], "tare_g": 260, "max_kg": 15, "cost_minor": 85},
        {"id": "L", "mm": [700, 450, 450], "tare_g": 520, "max_kg": 25, "cost_minor": 160}
    ])
}

fn measures(edges: &Value) -> Value {
    json!({
        "length": edges[0].to_string(),
        "width": edges[1].to_string(),
        "height": edges[2].to_string(),
    })
}

/// A commerce call takes the document and the request in one envelope.
fn commerce_call(request: Value) -> String {
    json!({"document": commerce_document(), "request": request}).to_string()
}

/// The service the customer chose, or ground when a rule forbids it, and why.
fn allowed_service(order: &Order) -> Result<(&'static str, Option<String>), Box<dyn Error>> {
    let products = products();
    for (sku, _) in order.lines {
        let battery = products[sku]["battery"].as_str().unwrap_or("none");
        let answer: Value = serde_json::from_str(&evaluate_policy_json(&commerce_call(json!({
            "scope": "service",
            "context": {"service": order.service, "battery": battery},
            "as_of": 0
        })))?)?;
        let decision = &answer["decision"];
        if decision["allowed"] == false {
            let reason = decision["citation"]["reason"].as_str().unwrap_or_default();
            return Ok(("ground", Some(format!("{sku}: {reason}"))));
        }
    }
    Ok((order.service, None))
}

fn packing_request(order: &Order) -> Value {
    let products = products();
    let items: Vec<Value> = order
        .lines
        .iter()
        .map(|(sku, quantity)| {
            let product = &products[sku];
            let mut item = json!({
                "id": sku,
                "quantity": quantity,
                "weight": format!("{} g", product["g"]),
                "dimensions": measures(&product["mm"]),
            });
            if let (Some(item), Some(rules)) = (item.as_object_mut(), product["rules"].as_object())
            {
                item.extend(rules.clone());
            }
            item
        })
        .collect();
    let containers: Vec<Value> = cartons()
        .as_array()
        .into_iter()
        .flatten()
        .map(|carton| {
            json!({
                "id": carton["id"],
                "inner_dimensions": measures(&carton["mm"]),
                "cost_minor": carton["cost_minor"],
                "tare_weight": format!("{} g", carton["tare_g"]),
                "max_payload": format!("{} kg", carton["max_kg"]),
            })
        })
        .collect();
    json!({
        "units": {"length": "mm"},
        "configuration": {
            "objective": "lowest_cost",
            // Nothing may hang in the air: three quarters of every base must rest on something.
            "minimum_support_ratio": 0.75,
            "effort_budget": {"max_candidates_evaluated": 200000},
            "time_limit_ms": 60000
        },
        "items": items,
        "containers": containers
    })
}

/// One parcel through the pinned tariff. Whole grams and millimetres, rounded up. The answer
/// is either a quote or a rejection with a code: a zone the tariff does not price is never a
/// zero.
fn price(container: &Value, service: &str, zone: &str) -> Result<Value, Box<dyn Error>> {
    let ticks = |measure: &Value| measure["ticks"].as_u64().unwrap_or(0);
    let grams = ticks(&container["gross_weight"]).div_ceil(TICKS_PER_GRAM);
    let volume: u64 = ["length", "width", "height"]
        .iter()
        .map(|edge| ticks(&container["outer_dimensions"][edge]).div_ceil(TICKS_PER_MM))
        .product();
    Ok(serde_json::from_str(&quote_json(&commerce_call(json!({
        "carrier_id": "postway", "service_id": service, "tariff_version": 1,
        "zone": zone, "actual_weight_g": grams, "volume_mm3": volume
    })))?)?)
}

fn fulfil(order: &Order) -> Result<(), Box<dyn Error>> {
    println!(
        "\norder {} to zone {}, customer chose {}",
        order.id, order.zone, order.service
    );
    let (service, why) = allowed_service(order)?;
    if let Some(why) = why {
        println!("  moved to {service}: {why}");
    }

    let request = packing_request(order).to_string();
    let text = match pack_json(&request) {
        Ok(text) => text,
        // `field` is a JSON Pointer into the request, `reason` one of a closed set.
        Err(PackError::InvalidRequest(error)) => {
            println!(
                "  on hold: {} at {} ({})",
                error.reason(),
                error.field(),
                error.detail()
            );
            return Ok(());
        }
        Err(other) => return Err(other.into()),
    };

    let issues = validate_json(&request, &text)?;
    if !issues.is_empty() {
        let found: Vec<String> = issues
            .iter()
            .map(|issue| format!("{}: {}", issue.code, issue.detail))
            .collect();
        println!("  on hold: the plan failed its check: {}", found.join("; "));
        return Ok(());
    }
    let result: Value = serde_json::from_str(&text)?;
    let containers = result["containers"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    println!("  plan checked: {} parcel(s), no issues", containers.len());

    let mut answers = Vec::with_capacity(containers.len());
    for container in containers {
        let answer = price(container, service, order.zone)?;
        if answer["status"] != "ok" {
            println!(
                "  on hold: no {service} price for zone {} ({})",
                order.zone,
                answer["error"]["code"].as_str().unwrap_or_default()
            );
            return Ok(());
        }
        answers.push(answer);
    }

    let mut total = 0;
    for (container, answer) in containers.iter().zip(&answers) {
        let mut contents: BTreeMap<&str, u32> = BTreeMap::new();
        for placement in container["placements"].as_array().into_iter().flatten() {
            *contents
                .entry(placement["item_type"].as_str().unwrap_or_default())
                .or_insert(0) += 1;
        }
        let listing: Vec<String> = contents
            .iter()
            .map(|(sku, count)| format!("{count} x {sku}"))
            .collect();
        let quoted = &answer["quote"];
        total += quoted["total_minor"].as_u64().unwrap_or(0);
        println!(
            "  {:<4} {}",
            container["id"].as_str().unwrap_or_default(),
            listing.join(", ")
        );
        println!(
            "       {} g on the scale, billed as {} g, {} minor units",
            quoted["actual_weight_g"], quoted["billed_weight_g"], quoted["total_minor"]
        );
    }
    println!("  shipping by {service}: {total} minor units");

    for entry in result["unpacked_items"].as_array().into_iter().flatten() {
        println!(
            "  {} goes to freight: {}",
            entry["item_id"].as_str().unwrap_or_default(),
            entry["reason"].as_str().unwrap_or_default()
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let orders = [
        Order {
            id: "A-1001",
            zone: "domestic",
            service: "air",
            lines: &[("coffee", 6), ("bleach", 2)],
        },
        Order {
            id: "A-1002",
            zone: "eu",
            service: "air",
            lines: &[("laptop", 1), ("monitor", 1), ("desk", 1)],
        },
        Order {
            id: "A-1003",
            zone: "domestic",
            service: "ground",
            lines: &[("coffee", 0)],
        },
        Order {
            id: "A-1004",
            zone: "overseas",
            service: "ground",
            lines: &[("coffee", 1)],
        },
    ];
    for order in &orders {
        fulfil(order)?;
    }
    Ok(())
}
