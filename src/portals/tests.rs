use super::*;
use mod_api::bindings::cinnabar::extension::terrain::{BlockState, ShapeBinding, Stamp};

fn rectangle(origin: [i32; 3], width: i32, height: i32, axis: Axis) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for u in 0..width {
        for y in 0..height {
            let mut cell = origin;
            cell[if axis == Axis::X { 0 } else { 2 }] += u;
            cell[1] += y;
            cells.push(cell);
        }
    }
    cells
}

fn sample(portal: Portal) -> Region {
    let query = portal.query();
    let size = [
        query.max.x - query.min.x,
        query.max.y - query.min.y,
        query.max.z - query.min.z,
    ];
    let mut states = Vec::new();
    for x in query.min.x..query.max.x {
        for z in query.min.z..query.max.z {
            for y in query.min.y..query.max.y {
                let p = [x, y, z];
                states.push(u16::from(
                    (0..3).all(|i| p[i] >= portal.min[i] && p[i] < portal.max[i]),
                ));
            }
        }
    }
    let block = |id: &str, canonical: Option<String>| BlockState {
        runtime_id: 1,
        identifier: Some(id.into()),
        canonical_state: canonical,
        tags: Vec::new(),
        physics: None,
        collision: ShapeBinding::NotRequested,
        selection: ShapeBinding::NotRequested,
    };
    let axis = if portal.axis == Axis::X { "x" } else { "z" };
    let cells = (size[0] * size[1] * size[2]) as usize;
    Region {
        stamp: Stamp {
            session: 1,
            dimension: 0,
            registry: 1,
            revision: 1,
        },
        chunks: Vec::new(),
        layer_count: 1,
        loaded: vec![u64::MAX; cells.div_ceil(64)],
        states,
        palette: vec![
            block("minecraft:air", None),
            block(
                PORTAL_ID,
                Some(format!(
                    r#"{{"portal_axis":{{"type":"string","value":"{axis}"}}}}"#
                )),
            ),
        ],
        shape_offsets: vec![0],
        shape_boxes: Vec::new(),
        overrides: Vec::new(),
    }
}

#[test]
fn bounded_detection_finishes_across_frames_without_publishing_partial_openings() {
    let points = rectangle([0, 60, 0], MAX_SIDE, MAX_SIDE, Axis::X);
    let expected = detect(&points);
    let mut detector = Detector::new(&points);
    assert!(detector.step(0).is_none());
    for _ in 0..points.len() * 2 {
        assert!(detector.step(1).is_none());
    }
    assert_eq!(detector.step(1), Some(expected));
}

#[test]
fn both_orientations_negative_coordinates_and_separate_frames_are_detected() {
    let mut points = rectangle([-14, 63, -2], 2, 3, Axis::X);
    points.extend(rectangle([10, 70, 12], 4, 6, Axis::Z));
    let portals = detect(&points);
    assert_eq!(portals.len(), 2);
    assert_eq!(portals[0].axis, Axis::X);
    assert_eq!(portals[0].center(), [-13.0, 64.5, -1.5]);
    assert_eq!(portals[1].axis, Axis::Z);
    for portal in portals {
        assert!(verify(portal, &sample(portal)));
        let query = portal.query();
        for extent in [
            query.max.x - query.min.x,
            query.max.y - query.min.y,
            query.max.z - query.min.z,
        ] {
            assert!((1..=mod_api::MAX_TERRAIN_EXTENT as i32).contains(&extent));
        }
    }
}

#[test]
fn holes_partial_strips_intersections_and_oversized_openings_are_rejected() {
    let mut hole = rectangle([0, 60, 0], 3, 4, Axis::X);
    hole.remove(5);
    assert!(detect(&hole).is_empty());
    assert!(detect(&rectangle([0, 60, 0], 1, 3, Axis::X)).is_empty());
    assert!(detect(&rectangle([0, 60, 0], MAX_SIDE + 1, 3, Axis::X)).is_empty());
    let mut crossing = rectangle([0, 60, 0], 3, 4, Axis::X);
    crossing.extend(rectangle([0, 60, 0], 3, 4, Axis::Z));
    assert!(detect(&crossing).is_empty());
    let maximum = detect(&rectangle([0, 60, 0], MAX_SIDE, MAX_SIDE, Axis::X));
    assert_eq!(maximum.len(), 1);
    assert!(verify(maximum[0], &sample(maximum[0])));
}

#[test]
fn terrain_verification_refuses_unknown_cells_truncated_results_and_wrong_axes() {
    let portal = detect(&rectangle([0, 60, 0], 2, 3, Axis::X))[0];
    let mut region = sample(portal);
    region.loaded[0] &= !1;
    assert!(
        !verify(portal, &region),
        "unknown border must not count as air"
    );
    region = sample(portal);
    region.states[0] = 1;
    assert!(
        !verify(portal, &region),
        "search truncation must not create a smaller window"
    );
    region = sample(portal);
    region.palette[1].canonical_state = Some(r#"{"portal_axis":{"value":"z"}}"#.into());
    assert!(!verify(portal, &region));
    region = sample(portal);
    region.states.clear();
    assert!(!verify(portal, &region));
    region = sample(portal);
    let interior = region.states.iter().position(|&v| v == 1).unwrap();
    region.states[interior] = 0;
    assert!(!verify(portal, &region), "destroyed portal must disappear");
}
