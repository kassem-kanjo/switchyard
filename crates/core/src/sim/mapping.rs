//! Destination-to-chute mapping strategies (`chutes.mapping`).

use super::chute_id;
use crate::config::ChuteMapping;
use crate::guidance::DistanceTables;
use crate::ids::ChuteId;
use crate::layout::Layout;
use crate::world::Goal;

/// The chute for each of `n` destinations under `mapping`. Destination ids
/// are popularity ranks (Zipf weight falls with the id), so "most popular"
/// means "lowest id".
pub(super) fn map_destinations(
    n: usize,
    mapping: ChuteMapping,
    layout: &Layout,
    distances: &DistanceTables,
) -> Vec<ChuteId> {
    let chutes = layout.chutes.len();
    let mut order: Vec<ChuteId> = (0..chutes).map(chute_id).collect();
    if mapping == ChuteMapping::PopularityNearInduction {
        // Sum of guided distances from every induction cell; ties by id.
        order.sort_by_key(|&c| {
            let total: u64 = layout
                .stations
                .iter()
                .map(|&s| u64::from(distances.get(Goal::Chute(c), s)))
                .sum();
            (total, c)
        });
    }
    (0..n).map(|d| order[d % chutes]).collect()
}
