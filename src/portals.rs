use mod_api::bindings::cinnabar::extension::terrain::{Geometry, Position, Region, RegionQuery};
use std::collections::HashSet;

pub const PORTAL_ID: &str = "minecraft:portal";
pub const VISIBLE_PORTALS: usize = 2;
// A one-cell verification collar must fit the host's region extent.
pub const MAX_SIDE: i32 = mod_api::MAX_TERRAIN_EXTENT as i32 - 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    X,
    Z,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Portal {
    pub min: [i32; 3],
    pub max: [i32; 3],
    pub axis: Axis,
}

impl Portal {
    pub fn width_axis(self) -> usize {
        if self.axis == Axis::X { 0 } else { 2 }
    }

    pub fn center(self) -> [f32; 3] {
        self.center64().map(|value| value as f32)
    }

    pub fn center64(self) -> [f64; 3] {
        std::array::from_fn(|i| (f64::from(self.min[i]) + f64::from(self.max[i])) * 0.5)
    }

    pub fn distance_squared(self, eye: [f32; 3]) -> f32 {
        self.center()
            .into_iter()
            .zip(eye)
            .map(|(a, b)| (a - b).powi(2))
            .sum()
    }

    /// Samples the opening and a complete border in its own plane, never more than 16² cells.
    pub fn query(self) -> RegionQuery {
        let mut min = self.min;
        let mut max = self.max;
        min[1] -= 1;
        max[1] += 1;
        min[self.width_axis()] -= 1;
        max[self.width_axis()] += 1;
        RegionQuery {
            min: Position {
                x: min[0],
                y: min[1],
                z: min[2],
            },
            max: Position {
                x: max[0],
                y: max[1],
                z: max[2],
            },
            geometry: Geometry::empty(),
        }
    }
}

/// A connected-component scan whose seeding and flood fill both have per-frame work limits.
pub struct Detector {
    points: Vec<[i32; 3]>,
    inserted: usize,
    remaining: HashSet<[i32; 3]>,
    stack: Vec<[i32; 3]>,
    component: Option<([i32; 3], [i32; 3], i64)>,
    result: Vec<Portal>,
}

impl Detector {
    pub fn new(points: &[[i32; 3]]) -> Self {
        Self {
            points: points.to_vec(),
            inserted: 0,
            remaining: HashSet::with_capacity(points.len()),
            stack: Vec::new(),
            component: None,
            result: Vec::new(),
        }
    }

    /// Returns only complete results, processing at most `work` insertions or cells per call.
    pub fn step(&mut self, mut work: usize) -> Option<Vec<Portal>> {
        while work > 0 {
            work -= 1;
            if let Some(&point) = self.points.get(self.inserted) {
                self.remaining.insert(point);
                self.inserted += 1;
                continue;
            }
            if self.stack.is_empty() {
                if let Some((min, max, count)) = self.component.take()
                    && let Some(portal) = complete_rectangle(min, max, count)
                {
                    self.result.push(portal);
                }
                let Some(&seed) = self.remaining.iter().next() else {
                    self.result.sort_by_key(|p| (p.min, p.max));
                    return Some(std::mem::take(&mut self.result));
                };
                self.remaining.remove(&seed);
                self.stack.push(seed);
                self.component = Some((seed, seed, 0));
            }
            let point = self.stack.pop().unwrap();
            let (min, max, count) = self.component.as_mut().unwrap();
            *count += 1;
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
                for delta in [-1, 1] {
                    if let Some(value) = point[axis].checked_add(delta) {
                        let mut neighbour = point;
                        neighbour[axis] = value;
                        if self.remaining.remove(&neighbour) {
                            self.stack.push(neighbour);
                        }
                    }
                }
            }
        }
        None
    }
}

/// Rejects holes, intersections, partial strips and frames that exceed the verification extent.
fn complete_rectangle(min: [i32; 3], max: [i32; 3], count: i64) -> Option<Portal> {
    let span: [i64; 3] = std::array::from_fn(|i| i64::from(max[i]) - i64::from(min[i]) + 1);
    let axis = if span[2] == 1 {
        Axis::X
    } else if span[0] == 1 {
        Axis::Z
    } else {
        return None;
    };
    let width = if axis == Axis::X { span[0] } else { span[2] };
    if !(2..=i64::from(MAX_SIDE)).contains(&width)
        || !(3..=i64::from(MAX_SIDE)).contains(&span[1])
        || count != width * span[1]
        || min.iter().any(|&v| v <= i32::MIN + 1)
        || max.iter().any(|&v| v >= i32::MAX - 1)
    {
        return None;
    }
    Some(Portal {
        min,
        max: max.map(|v| v + 1),
        axis,
    })
}

#[cfg(test)]
fn detect(points: &[[i32; 3]]) -> Vec<Portal> {
    Detector::new(points).step(usize::MAX).unwrap()
}

/// A search may be incomplete: accept an aperture only after loaded terrain confirms its border.
pub fn verify(portal: Portal, region: &Region) -> bool {
    let query = portal.query();
    let min = [query.min.x, query.min.y, query.min.z];
    let size = [
        query.max.x - query.min.x,
        query.max.y - query.min.y,
        query.max.z - query.min.z,
    ];
    let axes: Vec<_> = region
        .palette
        .iter()
        .map(|entry| {
            if entry.identifier.as_deref() != Some(PORTAL_ID) {
                return None;
            }
            let state: serde_json::Value =
                serde_json::from_str(entry.canonical_state.as_deref()?).ok()?;
            match state.get("portal_axis")?.get("value")?.as_str()? {
                "x" => Some(Axis::X),
                "z" => Some(Axis::Z),
                _ => None,
            }
        })
        .collect();
    for x in 0..size[0] {
        for z in 0..size[2] {
            for y in 0..size[1] {
                let cell = ((x * size[2] + z) * size[1] + y) as usize;
                if region
                    .loaded
                    .get(cell / 64)
                    .is_none_or(|bits| bits & (1 << (cell % 64)) == 0)
                {
                    return false;
                }
                let Some(&palette) = region.states.get(cell) else {
                    return false;
                };
                let Some(state) = region.palette.get(palette as usize) else {
                    return false;
                };
                let point = [x + min[0], y + min[1], z + min[2]];
                let inside = (0..3).all(|i| point[i] >= portal.min[i] && point[i] < portal.max[i]);
                if inside {
                    if axes.get(palette as usize).copied().flatten() != Some(portal.axis) {
                        return false;
                    }
                } else if state.identifier.as_deref() == Some(PORTAL_ID) {
                    return false;
                }
            }
        }
    }
    region.layer_count > 0
}

#[cfg(test)]
mod tests;
