//! Load propagation through the contact graph: how much weight each box carries and
//! whether the bearing rules hold. It sits below the solvers; the solvers and request admission
//! both consume it (R-02).

use crate::contact_graph::ContactGraph;
use crate::geometry::Aabb;
use crate::model::Placement;
use std::cmp::Reverse;

/// The item facts the bearing rules read about one box, and nothing else. `Placement`
/// answers from its item; `LoadBody` is the same facts copied or borrowed out once per
/// sweep, so a candidate check never clones an `Item` -- its id, tags and metadata -- to
/// read them.
pub(crate) trait LoadSubject {
    fn envelope(&self) -> Aabb;
    fn weight_ticks(&self) -> i64;
    fn max_top_load_ticks(&self) -> Option<i64>;
    fn max_stacked_items(&self) -> Option<usize>;
    fn max_compression_pressure_kpa(&self) -> Option<i64>;
    /// The item's own footprint, which a load presses on; the envelope is grown by the
    /// clearance and carries the support graph.
    fn footprint(&self) -> i128;
    fn stacking_group(&self) -> Option<&str>;
    /// The code refusing an item of `upper_group` resting directly on this box.
    fn stacking_refusal(&self, upper_group: Option<&str>) -> Option<&'static str>;
}

impl LoadSubject for Placement {
    fn envelope(&self) -> Aabb {
        self.envelope_box()
    }
    fn footprint(&self) -> i128 {
        self.dimensions.base_area()
    }
    fn weight_ticks(&self) -> i64 {
        self.instance.item.weight.0
    }
    fn max_top_load_ticks(&self) -> Option<i64> {
        self.instance.item.max_top_load.map(|weight| weight.0)
    }
    fn max_stacked_items(&self) -> Option<usize> {
        self.instance.item.max_stacked_items
    }
    fn max_compression_pressure_kpa(&self) -> Option<i64> {
        self.instance.item.max_compression_pressure_kpa
    }
    fn stacking_group(&self) -> Option<&str> {
        self.instance.item.stacking_group.as_deref()
    }
    fn stacking_refusal(&self, upper_group: Option<&str>) -> Option<&'static str> {
        self.instance.item.stacking_refusal(upper_group)
    }
}

pub fn calculate_top_loads(placements: &[Placement]) -> Option<Vec<i128>> {
    let graph = ContactGraph::from_placements(placements);
    calculate_top_loads_with_graph(placements, &graph)
}

pub(crate) fn calculate_top_loads_with_graph<T: LoadSubject>(
    bodies: &[T],
    graph: &ContactGraph,
) -> Option<Vec<i128>> {
    let mut loads = vec![0_i128; bodies.len()];
    let mut order = (0..bodies.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| {
        let envelope = bodies[*index].envelope();
        Reverse((envelope.z2(), envelope.origin.z))
    });

    for upper_index in order {
        let upper = bodies[upper_index].envelope();
        if upper.origin.z == 0 {
            continue;
        }
        let supports = graph.supporters(upper_index);
        if supports.is_empty() {
            continue;
        }
        let total_area = supports.iter().map(|edge| edge.area).sum::<i128>();
        let downward = i128::from(bodies[upper_index].weight_ticks()) + loads[upper_index];
        let mut distributed = 0_i128;
        for (position, edge) in supports.iter().enumerate() {
            let share = if position + 1 == supports.len() {
                downward.saturating_sub(distributed)
            } else {
                downward.saturating_mul(edge.area) / total_area
            };
            distributed = distributed.saturating_add(share);
            let support = &bodies[edge.index];
            // `stackable: false` is geometry, not load: nothing may rest on the item at
            // all. Gating it on `share > 0` meant a weightless box resting on a
            // non-stackable one transferred nothing and so was waved through, and since
            // nothing here self-validates the answer came back `feasible` while the
            // shared validator called it `non_stackable`.
            if support
                .stacking_refusal(bodies[upper_index].stacking_group())
                .is_some()
            {
                return None;
            }
            loads[edge.index] = loads[edge.index].saturating_add(share);
            if let Some(maximum) = support.max_top_load_ticks()
                && loads[edge.index] > i128::from(maximum)
            {
                return None;
            }
        }
    }
    Some(loads)
}
