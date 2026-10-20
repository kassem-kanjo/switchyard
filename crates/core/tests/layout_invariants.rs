//! Layout generator invariants, checked on the `small` preset and a range of
//! floor sizes.

use switchyard_core::config::{ConfigError, FloorConfig, ScenarioConfig};
use switchyard_core::grid::{Cell, Pos};
use switchyard_core::ids::ChuteId;
use switchyard_core::layout::{CHUTE_PITCH, Layout, UNREACHABLE, generate};
use switchyard_core::presets::SMALL_TOML;

fn floors() -> Vec<FloorConfig> {
    let mut v = vec![ScenarioConfig::from_toml_str(SMALL_TOML).unwrap().floor];
    for (width, height, stations, south_margin) in [
        (10, 10, 1, 1),
        (16, 20, 3, 3),
        (33, 31, 5, 4),
        (60, 48, 12, 6),
        (101, 77, 20, 8),
    ] {
        v.push(FloorConfig {
            width,
            height,
            stations,
            south_margin,
        });
    }
    v
}

fn each_layout(mut check: impl FnMut(&FloorConfig, &Layout)) {
    for f in floors() {
        let layout = generate(&f).unwrap_or_else(|e| panic!("{f:?}: {e}"));
        check(&f, &layout);
    }
}

#[test]
fn walls_surround_the_floor() {
    each_layout(|f, l| {
        let g = &l.grid;
        for x in 0..f.width {
            assert_eq!(g.at(Pos::new(x, 0)), Cell::Wall);
            assert_eq!(g.at(Pos::new(x, f.height - 1)), Cell::Wall);
        }
        for y in 0..f.height {
            assert_eq!(g.at(Pos::new(0, y)), Cell::Wall);
            assert_eq!(g.at(Pos::new(f.width - 1, y)), Cell::Wall);
        }
    });
}

#[test]
fn stations_sit_on_the_south_edge_and_are_reachable_from_every_free_cell() {
    each_layout(|f, l| {
        assert_eq!(l.stations.len(), usize::from(f.stations));
        let g = &l.grid;
        for (i, &s) in l.stations.iter().enumerate() {
            let p = g.pos(s);
            assert_eq!(p.y, f.height - 2, "station {i} not on the south row");
            assert!(matches!(g.cell(s), Cell::Induction(id) if id.index() == i));
            let dist = l.distance_field(&[s]);
            for (c, cell) in g.cells().iter().enumerate() {
                if *cell == Cell::Free {
                    assert_ne!(
                        dist[c], UNREACHABLE,
                        "{f:?}: station {i} unreachable from {c}"
                    );
                }
            }
        }
    });
}

#[test]
fn every_chute_has_a_reachable_adjacent_free_cell() {
    each_layout(|f, l| {
        assert!(!l.chutes.is_empty());
        let robot_cell = l.spawn_cells(1).expect("room for a robot")[0];
        for c in 0..l.chutes.len() {
            let id = ChuteId(u16::try_from(c).unwrap());
            let access = l.chute_access(id);
            assert!(!access.is_empty(), "{f:?}: chute {c} has no free neighbour");
            let dist = l.distance_field(&access);
            assert_ne!(
                dist[robot_cell as usize], UNREACHABLE,
                "{f:?}: chute {c} unreachable"
            );
        }
    });
}

#[test]
fn chutes_form_a_lattice_with_two_cell_aisles() {
    each_layout(|_, l| {
        let g = &l.grid;
        for &c in &l.chutes {
            let p = g.pos(c);
            assert_eq!(p.x % CHUTE_PITCH, 0);
            assert_eq!(p.y % CHUTE_PITCH, 0);
            // The 8 surrounding cells are open floor: holes never touch,
            // so aisles are two cells wide.
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let q = Pos::new(
                        u16::try_from(i32::from(p.x) + dx).unwrap(),
                        u16::try_from(i32::from(p.y) + dy).unwrap(),
                    );
                    assert_eq!(g.at(q), Cell::Free, "cell {q:?} next to chute {p:?}");
                }
            }
        }
        l.check_invariants().unwrap();
    });
}

#[test]
fn rejects_floors_that_cannot_hold_the_request() {
    let base = FloorConfig {
        width: 20,
        height: 20,
        stations: 2,
        south_margin: 3,
    };
    assert!(matches!(
        generate(&FloorConfig {
            stations: 0,
            ..base.clone()
        }),
        Err(ConfigError::BadStationCount { .. })
    ));
    assert!(matches!(
        generate(&FloorConfig {
            stations: 50,
            ..base.clone()
        }),
        Err(ConfigError::BadStationCount { .. })
    ));
    assert_eq!(
        generate(&FloorConfig {
            south_margin: 30,
            ..base
        }),
        Err(ConfigError::NoChutes)
    );
}
