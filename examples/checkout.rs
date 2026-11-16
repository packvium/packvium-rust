//! A shop's checkout: turn a cart into parcels.
//!
//!     cargo run --example checkout
//!
//! This is the shape most projects start from. You already have product records and cart
//! lines as your own types; the engine wants a JSON request. So there are two small
//! functions between them: `to_request` maps your types onto the request, and `parcels`
//! reads the result back into a struct your handler can return. Everything
//! Packvium-specific lives in those two functions, and the rest of the service never sees
//! a tick or a placement.
//!
//! `pack_json` is a plain synchronous function over text, so it sits behind an axum or
//! actix handler, or inside a queue worker, without any setup.

use std::collections::BTreeMap;

use packvium_core::pack_json;
use serde::Deserialize;
use serde_json::{Value, json};

/// A row of your product table. Sizes in centimetres and weights in grams, as a shop
/// usually has them.
struct Product {
    sku: &'static str,
    size_cm: [u32; 3],
    weight_g: u32,
    this_side_up: bool,
}

/// A carton on your packing bench, with what it costs you in cents.
struct Carton {
    id: &'static str,
    inner_cm: [u32; 3],
    max_kg: u32,
    cost_cents: u64,
}

const PRODUCTS: [Product; 4] = [
    Product {
        sku: "kettle",
        size_cm: [24, 22, 26],
        weight_g: 1300,
        this_side_up: true,
    },
    Product {
        sku: "mug",
        size_cm: [12, 9, 10],
        weight_g: 350,
        this_side_up: false,
    },
    Product {
        sku: "cookbook",
        size_cm: [28, 22, 3],
        weight_g: 900,
        this_side_up: false,
    },
    Product {
        sku: "floor-lamp",
        size_cm: [150, 30, 30],
        weight_g: 4200,
        this_side_up: false,
    },
];

const CARTONS: [Carton; 3] = [
    Carton {
        id: "small",
        inner_cm: [30, 25, 15],
        max_kg: 5,
        cost_cents: 45,
    },
    Carton {
        id: "medium",
        inner_cm: [40, 30, 30],
        max_kg: 12,
        cost_cents: 80,
    },
    Carton {
        id: "large",
        inner_cm: [60, 40, 40],
        max_kg: 20,
        cost_cents: 140,
    },
];

/// The part of a result this program reads. Unknown fields are ignored, so a result that
/// gains a field in a later release still deserializes.
#[derive(Deserialize)]
struct Outcome {
    containers: Vec<Packed>,
    unpacked_items: Vec<Unpacked>,
}

#[derive(Deserialize)]
struct Packed {
    container_type: String,
    gross_weight: Measure,
    placements: Vec<Placement>,
}

#[derive(Deserialize)]
struct Placement {
    item_type: String,
}

#[derive(Deserialize)]
struct Measure {
    value: String,
    unit: String,
}

#[derive(Deserialize)]
struct Unpacked {
    item_type: String,
}

/// What the checkout page needs: one entry per parcel, and what fits no carton.
struct Parcel {
    carton: String,
    weight: String,
    contents: BTreeMap<String, u32>,
}

fn measures(edges: [u32; 3]) -> Value {
    json!({
        "length": edges[0].to_string(),
        "width": edges[1].to_string(),
        "height": edges[2].to_string(),
    })
}

fn product(sku: &str) -> &'static Product {
    PRODUCTS
        .iter()
        .find(|product| product.sku == sku)
        .expect("the cart names a known SKU")
}

/// Your cart (SKU and quantity pairs) as a Packvium request.
fn to_request(cart: &[(&str, u32)]) -> Value {
    let items: Vec<Value> = cart
        .iter()
        .map(|&(sku, quantity)| {
            let product = product(sku);
            json!({
                "id": sku,
                "quantity": quantity,
                "weight": format!("{} g", product.weight_g),
                "keep_upright": product.this_side_up,
                "dimensions": measures(product.size_cm),
            })
        })
        .collect();
    let containers: Vec<Value> = CARTONS
        .iter()
        .map(|carton| {
            json!({
                "id": carton.id,
                "max_payload": format!("{} kg", carton.max_kg),
                "cost_minor": carton.cost_cents,
                "inner_dimensions": measures(carton.inner_cm),
            })
        })
        .collect();
    json!({
        "units": { "length": "cm" },
        "configuration": {
            // Pay as little as possible for cartons, after packing everything that fits.
            "objective": "lowest_cost",
            // Counted work decides where the search stops, so the same cart always gets the
            // same parcels. The time limit is a fuse against a hang, not a search limit.
            "effort_budget": { "max_candidates_evaluated": 200000 },
            "time_limit_ms": 60000
        },
        "items": items,
        "containers": containers
    })
}

/// The parcels to ship, and the SKUs that fit no carton you stock.
fn parcels(cart: &[(&str, u32)]) -> Result<(Vec<Parcel>, Vec<String>), Box<dyn std::error::Error>> {
    let outcome: Outcome = serde_json::from_str(&pack_json(&to_request(cart).to_string())?)?;
    let shipped = outcome
        .containers
        .into_iter()
        .map(|packed| {
            let mut contents = BTreeMap::new();
            for placement in packed.placements {
                *contents.entry(placement.item_type).or_insert(0) += 1;
            }
            Parcel {
                carton: packed.container_type,
                // `value` is the exact weight rendered in `unit`; here that is whole grams.
                weight: format!("{} {}", packed.gross_weight.value, packed.gross_weight.unit),
                contents,
            }
        })
        .collect();
    // An item that fits nowhere is not an error. It comes back with a reason, and the rest
    // of the cart is still packed.
    let mut left_out: Vec<String> = outcome
        .unpacked_items
        .into_iter()
        .map(|unpacked| unpacked.item_type)
        .collect();
    left_out.sort();
    left_out.dedup();
    Ok((shipped, left_out))
}

fn show(title: &str, cart: &[(&str, u32)]) -> Result<(), Box<dyn std::error::Error>> {
    let (shipped, left_out) = parcels(cart)?;
    println!("\n{title}");
    let mut carton_cost = 0;
    for (index, parcel) in shipped.iter().enumerate() {
        let contents: Vec<String> = parcel
            .contents
            .iter()
            .map(|(sku, count)| format!("{count} x {sku}"))
            .collect();
        println!(
            "  parcel {}: {:<6} {:>7}  {}",
            index + 1,
            parcel.carton,
            parcel.weight,
            contents.join(", ")
        );
        carton_cost += CARTONS
            .iter()
            .find(|carton| carton.id == parcel.carton)
            .map_or(0, |carton| carton.cost_cents);
    }
    println!("  cartons cost {carton_cost} cents");
    for sku in left_out {
        println!("  {sku} fits none of our cartons: ship it in its own packaging");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    show("a small order", &[("mug", 2), ("cookbook", 1)])?;
    show(
        "a bigger order",
        &[("kettle", 4), ("mug", 6), ("cookbook", 4)],
    )?;
    show(
        "an order with something oversized",
        &[("floor-lamp", 1), ("mug", 4)],
    )?;
    Ok(())
}
