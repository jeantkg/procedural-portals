//! Last-observed portal associations. These are observations, not a vanilla exit resolver.
use crate::cache as api;
use crate::portals::{Axis, Portal};
use mod_api::bindings::cinnabar::extension::player_state::WorldObservation;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Endpoint {
    pub dimension: i32,
    pub portal: Portal,
    pub side: i8,
}

#[derive(Default)]
pub struct Links {
    scope: Option<(u64, String, i32)>,
    armed: Option<(Endpoint, u16)>,
    pending: Option<(Endpoint, u16)>,
    links: Vec<(Endpoint, Endpoint)>,
}

pub(crate) fn near(portal: Portal, feet: [f32; 3], normal_range: f32) -> bool {
    let normal = if portal.axis == Axis::X { 2 } else { 0 };
    (feet[normal] - portal.center()[normal]).abs() < normal_range
        && (feet[portal.width_axis()] >= portal.min[portal.width_axis()] as f32 - 0.5)
        && (feet[portal.width_axis()] <= portal.max[portal.width_axis()] as f32 + 0.5)
        && feet[1] >= portal.min[1] as f32 - 0.5
        && feet[1] <= portal.max[1] as f32
}

impl Links {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Retains associations across dimension-loading gaps, never across connection identities.
    pub fn observe_world(&mut self, world: &WorldObservation) {
        if self
            .scope
            .as_ref()
            .is_some_and(|(session, name, _)| *session != world.session || *name != world.world)
        {
            self.clear();
        }
        if self
            .scope
            .as_ref()
            .is_some_and(|(_, _, dimension)| *dimension != world.dimension)
        {
            self.pending = self.armed.take().filter(|(endpoint, _)| {
                (endpoint.dimension == 0 && world.dimension == 1)
                    || (endpoint.dimension == 1 && world.dimension == 0)
            });
        }
        if let Some((_, _, dimension)) = &mut self.scope {
            *dimension = world.dimension;
        } else {
            self.scope = Some((world.session, world.world.clone(), world.dimension));
        }
        for value in [&mut self.armed, &mut self.pending] {
            if let Some((_, age)) = value {
                *age = age.saturating_sub(1);
                if *age == 0 {
                    *value = None;
                }
            }
        }
    }

    /// Requires exactly one verified opening near the observed arrival/approach position.
    pub fn observe_portals(&mut self, dimension: i32, feet: [f32; 3], portals: &[Portal]) {
        let mut nearby = portals
            .iter()
            .copied()
            .filter(|portal| near(*portal, feet, 3.));
        let Some(portal) = nearby.next() else {
            return;
        };
        if nearby.next().is_some() {
            return;
        }
        let (side, distance) = crate::view::side_distance(portal, feet);
        if distance < 0.35 {
            return;
        }
        let endpoint = Endpoint {
            dimension,
            portal,
            side,
        };
        if let Some((source, _)) = self.pending.take() {
            self.links.retain(|(old, _)| {
                old.dimension != source.dimension || old.portal != source.portal
            });
            if self.links.len() == 16 {
                self.links.remove(0);
            }
            self.links.push((source, endpoint));
        }
        self.armed = Some((endpoint, 600));
    }

    pub fn destination(&self, dimension: i32, portal: Portal) -> Option<(Endpoint, Endpoint)> {
        self.links
            .iter()
            .copied()
            .find(|(source, _)| source.dimension == dimension && source.portal == portal)
    }

    pub fn invalidate(&mut self, dimension: i32, portal: Portal) {
        self.links
            .retain(|(source, _)| source.dimension != dimension || source.portal != portal);
    }
}

pub fn query(destination: Endpoint) -> api::Query {
    let center = destination.portal.center64();
    let extent = mod_api::MAX_TERRAIN_EXTENT;
    let (min_y, max_y) = match destination.dimension {
        0 => (-64, 320),
        1 => (0, 128),
        _ => (0, 256),
    };
    let min = center.map(|v| v.floor() as i32 - i32::from(extent) / 2);
    api::Query {
        dimension: destination.dimension,
        min: api::Position {
            x: min[0],
            y: min[1].clamp(min_y, max_y - i32::from(extent)),
            z: min[2],
        },
        size: (extent, extent, extent),
    }
}

/// Maps destination coordinates into the source dimension with a proper rotation, never a mirror.
pub fn rotation(source: Endpoint, destination: Endpoint) -> (f32, f32) {
    let angle = |axis| {
        if axis == Axis::X {
            0.
        } else {
            std::f32::consts::FRAC_PI_2
        }
    };
    let turn = if source.side == destination.side {
        std::f32::consts::PI
    } else {
        0.
    };
    let angle = angle(source.portal.axis) + turn - angle(destination.portal.axis);
    (angle.sin().round(), angle.cos().round())
}

/// Validates the destination opening against the fresh named-state sample, including its border.
pub fn destination_valid(destination: Endpoint, query: &api::Query, region: &api::Region) -> bool {
    let portal = destination.portal;
    let border = portal.query();
    let axis = match portal.axis {
        Axis::X => "x",
        Axis::Z => "z",
    };
    let matching: Vec<bool> = region
        .palette
        .iter()
        .map(|state| {
            state.name == crate::portals::PORTAL_ID
                && serde_json::from_str::<serde_json::Value>(&state.states)
                    .ok()
                    .is_some_and(|states| states["portal_axis"]["value"].as_str() == Some(axis))
        })
        .collect();
    for x in border.min.x..border.max.x {
        for z in border.min.z..border.max.z {
            for y in border.min.y..border.max.y {
                let [dx, dy, dz] = [x - query.min.x, y - query.min.y, z - query.min.z];
                if dx < 0
                    || dy < 0
                    || dz < 0
                    || dx >= i32::from(query.size.0)
                    || dy >= i32::from(query.size.1)
                    || dz >= i32::from(query.size.2)
                {
                    return false;
                }
                let index =
                    ((dx * i32::from(query.size.2) + dz) * i32::from(query.size.1) + dy) as usize;
                let Some(&palette) = region.cells.get(index) else {
                    return false;
                };
                let Some(state) = region.palette.get(palette as usize) else {
                    return false;
                };
                let inside = [x, y, z]
                    .iter()
                    .enumerate()
                    .all(|(i, value)| *value >= portal.min[i] && *value < portal.max[i]);
                if inside {
                    if !matching[palette as usize] {
                        return false;
                    }
                } else if state.name == crate::portals::PORTAL_ID {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn endpoint(axis: Axis, side: i8) -> Endpoint {
        Endpoint {
            dimension: 0,
            portal: Portal {
                min: [0, 64, 0],
                max: [2, 67, 1],
                axis,
            },
            side,
        }
    }

    #[test]
    fn every_rotation_maps_exit_to_the_far_side_without_reflection() {
        for a in [Axis::X, Axis::Z] {
            for b in [Axis::X, Axis::Z] {
                for sa in [-1, 1] {
                    for sb in [-1, 1] {
                        let (s, c) = rotation(endpoint(a, sa), endpoint(b, sb));
                        let n = if b == Axis::X { [0., 1.] } else { [1., 0.] };
                        let mapped = [n[0] * c + n[1] * s, -n[0] * s + n[1] * c];
                        let value = mapped[if a == Axis::X { 1 } else { 0 }];
                        assert_eq!(value * f32::from(sb), -f32::from(sa));
                        assert_eq!(s * s + c * c, 1.);
                    }
                }
            }
        }
    }

    #[test]
    fn destination_query_stays_within_dimension_height() {
        let mut dest = endpoint(Axis::X, 1);
        dest.dimension = 1;
        dest.portal.min[1] = 123;
        dest.portal.max[1] = 127;
        let query = query(dest);
        assert_eq!(query.min.y + i32::from(query.size.1), 128);
    }

    #[test]
    fn links_are_directional_and_never_survive_a_new_connection() {
        let mut links = Links::default();
        let mut world = WorldObservation {
            session: 2,
            world: "local:world".into(),
            dimension: 0,
            position: (1., 64., 1.5),
            alive: true,
            runtime_id: 99,
        };
        let source = endpoint(Axis::X, 1).portal;
        links.observe_world(&world);
        links.observe_portals(0, [1., 64., 1.5], &[source]);
        world.dimension = 1;
        links.observe_world(&world);
        links.observe_portals(1, [1., 64., 1.5], &[source]);
        assert!(links.destination(0, source).is_some());
        assert!(links.destination(1, source).is_none());
        world.session += 1;
        links.observe_world(&world);
        assert!(links.destination(0, source).is_none());
    }
}
