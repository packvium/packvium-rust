//! What a carrier bills a container for: the dimensional weight and the first container
//! a rate table cannot price. Kept beside the model's `RateTable`, below the solvers that
//! rank by it and the admission code that guards it (R-02).

use crate::model::{Container, PackedContainer, PackingConfig, RateTable};

pub(crate) fn dimensional_weight_ticks(container: &Container, config: &PackingConfig) -> i128 {
    // An empty pallet is billed at its declared footprint, never its solving frame.
    config.dimensional_weight_ticks(
        container
            .outer_dimensions
            .unwrap_or_else(|| container.declared_inner()),
    )
}

/// What a carrier bills by volume for this container as filled: a flat bag's achieved
/// package (empty: nothing), which only grows as items are added and so also bounds any
/// completion from below, else the declared outer size. `O(p)` for a bag.
pub(crate) fn billed_dimensional_weight(packed: &PackedContainer, config: &PackingConfig) -> i128 {
    config.dimensional_weight_ticks(packed.billed_dimensions())
}

/// The first container in a finished answer whose own rate table cannot price it, as
/// `(container id, billed grams, last bracket)`.
///
/// Ranking an unpriceable candidate worst is what makes a priceable alternative win the
/// round; it is a search device, not an answer. This is the guard that keeps the sentinel
/// from ever surfacing: a returned packing the tariff cannot price would quote a number
/// the carrier never published, which is the one outcome `charge_minor` refuses to
/// invent. Callers turn a hit into the same refusal Python and PHP raise.
pub(crate) fn unpriceable_container(
    containers: &[PackedContainer],
    config: &PackingConfig,
) -> Option<(String, i64, i64)> {
    if config.objective != "lowest_landed_cost" {
        return None;
    }
    containers.iter().find_map(|packed| {
        let dimensional = billed_dimensional_weight(packed, config);
        let billed = i128::from(packed.gross_weight().0).max(dimensional);
        let grams = RateTable::grams_saturating(billed);
        // A container with no table at all is refused at admission, so `parse_request`
        // callers never reach this arm -- but `pack_request_with_policy` is public, and a
        // Rust consumer assembling a `PackingRequest` by hand bypasses that check. Report
        // it rather than skipping it: an untabled container is the one case where "cannot
        // price" is certain. Python and PHP report the same `0` bracket here.
        let Some(table) = packed.container.rate_table.as_ref() else {
            return Some((packed.container.id.clone(), grams, 0));
        };
        if table.charge_minor(grams).is_some() {
            return None;
        }
        let last = table.weight_brackets_g.last().copied().unwrap_or(0);
        Some((packed.container.id.clone(), grams, last))
    })
}
