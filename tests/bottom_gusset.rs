//! 1.11: bottom gussets, lying flat or standing, admitted by the two loops of film.
//!
//! Mirrors `packvium-python/tests/test_bottom_gusset.py` through `pack_json`, `pack_request`
//! and `validate_json`; refusal and issue texts are asserted whole because they are compared
//! across engines.

use packvium_core::{
    BagOrientation, Container, DeformableBoundary, Dimensions, Item, Length, PackingConfig,
    PackingRequest, Rotation, Weight, pack_json, pack_request, validate_json,
};
use serde_json::{Value, json};

const MM: i64 = Length::TICKS_PER_MM;

fn mm(length: i64, width: i64, height: i64) -> Dimensions {
    Dimensions {
        length: Length(length * MM),
        width: Length(width * MM),
        height: Length(height * MM),
    }
}

fn request(boundary: Value, item: Value, quantity: u64) -> Value {
    let mut item = item;
    item["quantity"] = json!(quantity);
    item["weight"] = json!("300 g");
    json!({
        "items": [item],
        "containers": [{"id": "pouch", "quantity": 1, "deformable_boundary": boundary,
                        "inner_dimensions": {"length": "300", "width": "150", "height": "100"}}],
        "configuration": {"solver_profile": "fast", "time_limit_ms": 60000,
                          "effort_budget": {"max_search_nodes": 100000}}
    })
}

fn rod() -> Value {
    json!({"id": "rod", "allowed_rotations": ["LWH"],
           "dimensions": {"length": "300", "width": "100", "height": "20"}})
}

fn bottle() -> Value {
    json!({"id": "bottle", "allowed_rotations": ["LWH"],
           "dimensions": {"length": "50", "width": "50", "height": "150"}})
}

fn standing() -> Value {
    json!({"kind": "gusseted_bag", "bottom_gusset_depth": "60", "orientation": "standing"})
}

fn solve(request: &Value) -> Value {
    serde_json::from_str(&pack_json(&request.to_string()).expect("the request packs"))
        .expect("the result is JSON")
}

fn first_unpacked_reason(result: &Value) -> &str {
    result["unpacked_items"][0]["reason"]
        .as_str()
        .expect("an unpacked reason")
}

fn values(dimensions: &Value) -> [&str; 3] {
    ["length", "width", "height"]
        .map(|side| dimensions[side]["value"].as_str().expect("a measure value"))
}

#[test]
fn a_bottom_gusset_admits_a_rod_the_flat_length_refuses() {
    // a + c = 320 > 300 lying flat; a 40 mm bottom gusset makes the loop 2L + K = 640 =
    // 2(a + c), and equality fits.
    let flat = solve(&request(
        json!({"kind": "gusseted_bag", "side_gusset_depth": "10"}),
        rod(),
        1,
    ));
    assert_eq!(
        first_unpacked_reason(&flat),
        "no_compatible_container_dimensions"
    );
    let block = solve(&request(
        json!({"kind": "gusseted_bag", "bottom_gusset_depth": "40"}),
        rod(),
        1,
    ));
    assert_eq!(block["unpacked_items"], json!([]));
    let past = solve(&request(
        json!({"kind": "gusseted_bag",
               "bottom_gusset_depth": {"value": (40 * MM - 1).to_string(), "unit": "ticks"}}),
        rod(),
        1,
    ));
    assert_eq!(
        first_unpacked_reason(&past),
        "no_compatible_container_dimensions"
    );
}

#[test]
fn a_standing_pouch_packs_in_its_turned_frame_and_the_validator_agrees() {
    // Standing, the 300 x 150 x 100 pouch is a 150 x 100 x 300 frame: two bottles stand side
    // by side (a + b = 150 = W + G); a third breaks the loop round the section and one on top
    // the loop down the bag, 2(b + c) = 700 > 660.
    let body = request(standing(), bottle(), 3);
    let result = solve(&body);
    let packed = &result["containers"][0];
    assert_eq!(
        packed["placements"].as_array().expect("placements").len(),
        2
    );
    assert_eq!(
        result["unpacked_items"].as_array().expect("unpacked").len(),
        1
    );
    let [a, b, c] = values(&packed["achieved_dimensions"]);
    assert_eq!(c, "150", "the bottles stand up the bag");
    let mut section = [a, b];
    section.sort_unstable();
    assert_eq!(section, ["100", "50"], "two bottles side by side");
    assert_eq!(values(&packed["inner_dimensions"]), ["300", "150", "100"]);
    let issues = validate_json(&body.to_string(), &result.to_string()).expect("judged");
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn the_validator_refuses_a_standing_package_past_its_loop_and_says_which() {
    let body = request(standing(), bottle(), 2);
    let result = solve(&body);
    let [a, b, c] = values(&result["containers"][0]["achieved_dimensions"])
        .map(|value| value.parse::<i64>().expect("whole millimetres") * MM);
    let mut narrower = body;
    narrower["containers"][0]["inner_dimensions"]["width"] = json!("149");
    let issues = validate_json(&narrower.to_string(), &result.to_string()).expect("judged");
    let girth: Vec<_> = issues
        .iter()
        .filter(|issue| issue.code == "bag_girth_exceeded")
        .map(|issue| issue.detail.clone())
        .collect();
    assert_eq!(
        girth,
        [format!(
            "pouch#1: b + c = {}, a + b = {}, standing, length {}, width {}, bottom gusset {}",
            b + c,
            a + b,
            300 * MM,
            149 * MM,
            60 * MM
        )]
    );
}

#[test]
fn a_flat_breach_names_the_bottom_gusset_beside_the_side_one() {
    let body = request(
        json!({"kind": "gusseted_bag", "side_gusset_depth": "10", "bottom_gusset_depth": "40"}),
        rod(),
        1,
    );
    let result = solve(&body);
    let mut shorter = body;
    shorter["containers"][0]["deformable_boundary"]["bottom_gusset_depth"] = json!("39");
    let issues = validate_json(&shorter.to_string(), &result.to_string()).expect("judged");
    let girth: Vec<_> = issues
        .iter()
        .filter(|issue| issue.code == "bag_girth_exceeded")
        .map(|issue| issue.detail.clone())
        .collect();
    assert_eq!(
        girth,
        [format!(
            "pouch#1: a + c = {}, b + c = {}, flat size {} x {}, side gusset {}, bottom gusset {}",
            320 * MM,
            120 * MM,
            300 * MM,
            150 * MM,
            10 * MM,
            39 * MM
        )]
    );
}

#[test]
fn fixed_placements_are_given_in_the_standing_frame() {
    let fixed = json!([
        {"item_type": "bottle", "container_type": "pouch", "orientation": "LWH",
         "position": {"x": "0", "y": "0", "z": "0"}},
        {"item_type": "bottle", "container_type": "pouch", "orientation": "LWH",
         "position": {"x": "0", "y": "50", "z": "0"}}
    ]);
    let mut at = request(standing(), bottle(), 2);
    at["fixed_placements"] = fixed.clone();
    assert_eq!(solve(&at)["unpacked_items"], json!([]));
    let mut past = at;
    past["containers"][0]["inner_dimensions"]["width"] = json!("149");
    match pack_json(&past.to_string()) {
        Err(error) => assert_eq!(
            error.to_string(),
            "invalid_fixed_placement: bag_girth_exceeded: pouch#1"
        ),
        Ok(_) => panic!("the fixed set was admitted"),
    }
}

fn pouch(boundary: DeformableBoundary) -> Container {
    Container {
        deformable_boundary: Some(boundary),
        ..Container::new("pouch", mm(300, 150, 100))
    }
}

/// Depths in ticks, zero for a gusset not declared.
fn gusseted(side: i64, bottom: i64, orientation: BagOrientation) -> Container {
    pouch(DeformableBoundary::GussetedBag {
        side_gusset_depth: Length(side),
        bottom_gusset_depth: Length(bottom),
        orientation,
    })
}

fn typed(container: Container, quantity: usize) -> PackingRequest {
    PackingRequest {
        items: vec![Item {
            allowed_rotations: vec![Rotation::Lwh],
            ..Item::new("bottle", mm(50, 50, 150), Weight(300), quantity)
        }],
        containers: vec![container],
        config: PackingConfig::default(),
        output_length_unit: "mm".into(),
        output_weight_unit: "g".into(),
        catalog_versions_used: Vec::new(),
        currency: None,
        fixed_placements: Vec::new(),
        fixed_containers: Vec::new(),
    }
}

#[test]
fn the_object_api_refuses_what_the_json_path_refuses() {
    let cases = [
        (
            // Neither depth: refused as 1.10 refused a zero side gusset.
            gusseted(0, 0, BagOrientation::Flat),
            "container pouch side_gusset_depth must be positive",
        ),
        (
            gusseted(0, 600 * MM + 1, BagOrientation::Flat),
            "container pouch bottom_gusset_depth exceeds twice inner_dimensions length",
        ),
        (
            gusseted(10 * MM, -1, BagOrientation::Flat),
            "container pouch bottom_gusset_depth must be positive",
        ),
        (
            gusseted(10 * MM, 0, BagOrientation::Standing),
            "container pouch bottom_gusset_depth is required when orientation is standing",
        ),
        (
            gusseted(150 * MM + 1, 0, BagOrientation::Flat),
            "container pouch side_gusset_depth exceeds inner_dimensions width",
        ),
    ];
    for (container, detail) in cases {
        let error = pack_request(&typed(container, 1)).expect_err("refused");
        assert!(error.to_string().contains(detail), "{error} lacks {detail}");
    }
    let twice = gusseted(0, 600 * MM, BagOrientation::Flat);
    assert!(pack_request(&typed(twice, 1)).is_ok(), "K = 2L is admitted");
    let side_only = gusseted(10 * MM, 0, BagOrientation::Flat);
    assert!(
        pack_request(&typed(side_only, 1)).is_ok(),
        "K = 0 is no bottom gusset"
    );
}

#[test]
fn a_typed_standing_pouch_packs_in_its_frame_and_declares_the_bag_as_given() {
    let one = Container {
        quantity: Some(1),
        ..gusseted(0, 60 * MM, BagOrientation::Standing)
    };
    let result = pack_request(&typed(one, 3)).expect("packs");
    let packed = &result.containers[0];
    assert_eq!(packed.container.inner_dimensions, mm(150, 100, 300));
    assert_eq!(packed.container.declared_inner(), mm(300, 150, 100));
    assert!(packed.container.upright_frame);
    assert_eq!(packed.placements.len(), 2);
    assert_eq!(result.unpacked.len(), 1);
}

#[test]
fn the_loops_admit_equality_and_refuse_one_tick_past_in_either_frame() {
    // L = 300, W = 150, G = 10, K = 60: flat 2(a + c) <= 660 and b + c <= 160; standing, in
    // the 150 x 100 x 300 frame, 2(b + c) <= 660 and a + b <= 160.
    let flat = gusseted(10 * MM, 60 * MM, BagOrientation::Flat);
    assert!(flat.girth_admits([300 * MM, 130 * MM, 30 * MM]));
    assert!(!flat.girth_admits([300 * MM, 130 * MM, 30 * MM + 1]));
    assert!(!flat.girth_admits([200 * MM, 130 * MM + 1, 30 * MM]));
    let standing = Container {
        inner_dimensions: mm(150, 100, 300),
        upright_frame: true,
        ..gusseted(10 * MM, 60 * MM, BagOrientation::Standing)
    };
    assert_eq!(standing.declared_inner(), mm(300, 150, 100));
    assert!(standing.girth_admits([60 * MM, 100 * MM, 230 * MM]));
    assert!(!standing.girth_admits([60 * MM, 100 * MM, 230 * MM + 1]));
    assert!(!standing.girth_admits([60 * MM + 1, 100 * MM, 200 * MM]));
    assert!(standing.fits_alone(mm(50, 50, 150)));
    assert!(!standing.fits_alone(mm(50, 50, 300 + 1)));
}

#[test]
fn the_json_path_refuses_a_standing_bag_without_a_bottom_gusset() {
    let body = request(
        json!({"kind": "gusseted_bag", "side_gusset_depth": "10", "orientation": "standing"}),
        bottle(),
        1,
    );
    let error = pack_json(&body.to_string()).expect_err("refused");
    let refusal = error.request_error().expect("a structured refusal");
    assert_eq!(
        (refusal.reason(), refusal.field(), refusal.detail()),
        (
            "missing_field",
            "/containers/0/deformable_boundary/bottom_gusset_depth",
            "is required when orientation is standing"
        )
    );
}
