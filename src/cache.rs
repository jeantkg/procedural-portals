//! Connection-scoped snapshots of terrain the player has actually loaded.
//! These types belong to the mod, not to a new host interface.
use crate::{live::Endpoint, portals::Portal};
use mod_api::bindings::cinnabar::extension::{player_state::WorldObservation, terrain};
use std::{collections::HashMap, rc::Rc};

pub const UNKNOWN: u16 = u16::MAX;
pub const MAX_SNAPSHOTS: usize = 8;
const SLAB_HEIGHT: i32 = 1;
const MAX_SNAPSHOT_METADATA: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    pub dimension: i32,
    pub min: Position,
    pub size: (u8, u8, u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub session: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockState {
    pub runtime_id: u32,
    pub name: String,
    pub states: String,
}
#[derive(Clone, Debug)]
pub struct Region {
    pub stamp: Stamp,
    pub palette: Vec<BlockState>,
    pub cells: Vec<u16>,
}

struct Capture {
    slab: i32,
    palette: Vec<BlockState>,
    indices: HashMap<u32, u16>,
    cells: Vec<u16>,
    metadata: usize,
    changed: bool,
}
impl Capture {
    fn new(query: Query) -> Self {
        Self {
            slab: 0,
            palette: vec![],
            indices: HashMap::new(),
            metadata: 0,
            changed: false,
            cells: vec![
                UNKNOWN;
                usize::from(query.size.0)
                    * usize::from(query.size.1)
                    * usize::from(query.size.2)
            ],
        }
    }
}
struct Entry {
    query: Query,
    portal: Portal,
    region: Option<Rc<Region>>,
    capture: Capture,
}

#[derive(Default)]
pub struct Cache {
    scope: Option<(u64, String)>,
    registry: Option<u64>,
    revision: u64,
    entries: Vec<Entry>,
}

impl Cache {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A loading gap may retain snapshots; a different connection/world never may.
    pub fn observe_world(&mut self, world: &WorldObservation) {
        if !world.alive
            || self
                .scope
                .as_ref()
                .is_some_and(|(session, name)| *session != world.session || *name != world.world)
        {
            self.clear();
        }
        if world.alive && self.scope.is_none() {
            self.scope = Some((world.session, world.world.clone()));
        }
    }

    /// Runtime block IDs remain usable only with the exact observed registry generation.
    pub fn observe_registry(&mut self, registry: u64) -> bool {
        let changed = self.registry.is_some_and(|old| old != registry);
        if changed {
            self.entries.clear();
        }
        self.registry = Some(registry);
        changed
    }

    fn entry(&mut self, endpoint: Endpoint) -> usize {
        let query = crate::live::query(endpoint);
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.query == query && entry.portal == endpoint.portal)
        {
            return index;
        }
        if self.entries.len() == MAX_SNAPSHOTS {
            self.entries.remove(0);
        }
        self.entries.push(Entry {
            query,
            portal: endpoint.portal,
            region: None,
            capture: Capture::new(query),
        });
        self.entries.len() - 1
    }

    fn slab_query(entry: &Entry) -> terrain::RegionQuery {
        let query = entry.query;
        let y = query.min.y + entry.capture.slab;
        terrain::RegionQuery {
            min: terrain::Position {
                x: query.min.x,
                y,
                z: query.min.z,
            },
            max: terrain::Position {
                x: query.min.x + i32::from(query.size.0),
                y: (y + SLAB_HEIGHT).min(query.min.y + i32::from(query.size.1)),
                z: query.min.z + i32::from(query.size.2),
            },
            geometry: terrain::Geometry::empty(),
        }
    }

    /// One bounded slab per frame, using only the existing current-dimension terrain API.
    pub fn capture(&mut self, world: &WorldObservation, portal: Portal) -> bool {
        let endpoint = Endpoint {
            dimension: world.dimension,
            portal,
            side: 1,
        };
        let index = self.entry(endpoint);
        let query = Self::slab_query(&self.entries[index]);
        match terrain::read_region(query, None) {
            Ok(terrain::ReadResult::Changed(region))
                if region.stamp.session == world.session
                    && region.stamp.dimension == world.dimension =>
            {
                if self.registry != Some(region.stamp.registry) {
                    return true;
                }
                self.ingest(index, &region);
                true
            }
            Ok(terrain::ReadResult::Pending) | Err(terrain::ReadError::BudgetExhausted) => false,
            _ => {
                self.invalidate(index);
                true
            }
        }
    }

    fn invalidate(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.region = None;
        entry.capture = Capture::new(entry.query);
    }

    /// Copies primary layers only. Unloaded cells and missing metadata invalidate the snapshot.
    fn ingest(&mut self, index: usize, sample: &terrain::Region) {
        let entry = &mut self.entries[index];
        let slab = Self::slab_query(entry);
        let height = (slab.max.y - slab.min.y) as usize;
        let sx = usize::from(entry.query.size.0);
        let sy = usize::from(entry.query.size.1);
        let sz = usize::from(entry.query.size.2);
        let count = sx * height * sz;
        if sample.layer_count == 0 || sample.states.len() < count {
            self.invalidate(index);
            return;
        }
        let mut remap = vec![None; sample.palette.len()];
        for cell in 0..count {
            if sample
                .loaded
                .get(cell / 64)
                .is_none_or(|word| word & (1 << (cell % 64)) == 0)
            {
                self.invalidate(index);
                return;
            }
            let palette_index = sample.states[cell] as usize;
            let Some(state) = sample.palette.get(palette_index) else {
                self.invalidate(index);
                return;
            };
            let Some(name) = &state.identifier else {
                self.invalidate(index);
                return;
            };
            let palette = if let Some(palette) = remap[palette_index] {
                palette
            } else if let Some(&palette) = entry.capture.indices.get(&state.runtime_id) {
                remap[palette_index] = Some(palette);
                palette
            } else {
                let states = if name == crate::portals::PORTAL_ID {
                    state.canonical_state.as_deref().unwrap_or("{}")
                } else {
                    ""
                };
                entry.capture.metadata += name.len() + states.len();
                if entry.capture.metadata > MAX_SNAPSHOT_METADATA {
                    self.invalidate(index);
                    return;
                }
                let palette = entry.capture.palette.len() as u16;
                entry.capture.palette.push(BlockState {
                    runtime_id: state.runtime_id,
                    name: name.clone(),
                    states: states.into(),
                });
                entry.capture.indices.insert(state.runtime_id, palette);
                remap[palette_index] = Some(palette);
                palette
            };
            let x = cell / (sz * height);
            let z = (cell / height) % sz;
            let y = cell % height + entry.capture.slab as usize;
            let destination = (x * sz + z) * sy + y;
            entry.capture.changed |= entry.region.as_ref().is_none_or(|old| {
                old.palette[old.cells[destination] as usize].runtime_id != state.runtime_id
            });
            entry.capture.cells[destination] = palette;
        }
        entry.capture.slab += height as i32;
        if entry.capture.slab < i32::from(entry.query.size.1) {
            return;
        }
        let complete = std::mem::replace(&mut entry.capture, Capture::new(entry.query));
        // Compare while copying each slab, avoiding a full-volume equality scan in one callback.
        if entry.region.is_some() && !complete.changed {
            return;
        }
        let mut region = Region {
            stamp: Stamp {
                session: sample.stamp.session,
                revision: 0,
            },
            palette: complete.palette,
            cells: complete.cells,
        };
        if !crate::live::destination_valid(
            Endpoint {
                dimension: entry.query.dimension,
                portal: entry.portal,
                side: 1,
            },
            &entry.query,
            &region,
        ) {
            entry.region = None;
            return;
        }
        self.revision += 1;
        region.stamp.revision = self.revision;
        entry.region = Some(Rc::new(region));
    }

    /// Shares an immutable complete snapshot; never reinterprets missing cells as air.
    pub fn read(&self, query: Query) -> Option<Rc<Region>> {
        self.entries
            .iter()
            .find(|entry| entry.query == query)?
            .region
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portals::Axis;

    fn world() -> WorldObservation {
        WorldObservation {
            session: 2,
            world: "fixture".into(),
            dimension: 1,
            position: (1., 64., 1.5),
            alive: true,
            runtime_id: 1,
        }
    }
    fn endpoint() -> Endpoint {
        Endpoint {
            dimension: 1,
            side: 1,
            portal: Portal {
                min: [0, 64, 0],
                max: [2, 67, 1],
                axis: Axis::X,
            },
        }
    }
    fn sample(query: terrain::RegionQuery) -> terrain::Region {
        let state = |id, name: &str, canonical_state| terrain::BlockState {
            runtime_id: id,
            identifier: Some(name.into()),
            canonical_state,
            tags: vec![],
            physics: None,
            collision: terrain::ShapeBinding::NotRequested,
            selection: terrain::ShapeBinding::NotRequested,
        };
        let mut states = vec![];
        for x in query.min.x..query.max.x {
            for z in query.min.z..query.max.z {
                for y in query.min.y..query.max.y {
                    states.push(if (0..2).contains(&x) && z == 0 && (64..67).contains(&y) {
                        1
                    } else if x == 3 && z == 4 && y == 64 {
                        2
                    } else {
                        0
                    });
                }
            }
        }
        terrain::Region {
            stamp: terrain::Stamp {
                session: 2,
                dimension: 1,
                registry: 1,
                revision: 1,
            },
            chunks: vec![],
            layer_count: 1,
            loaded: vec![u64::MAX; states.len().div_ceil(64)],
            states,
            palette: vec![
                state(0, "minecraft:air", None),
                state(
                    1,
                    "minecraft:portal",
                    Some(r#"{"portal_axis":{"type":"string","value":"x"}}"#.into()),
                ),
                state(2, "minecraft:glowstone", None),
            ],
            shape_offsets: vec![],
            shape_boxes: vec![],
            overrides: vec![],
        }
    }
    fn populate(cache: &mut Cache) -> Query {
        cache.observe_world(&world());
        cache.observe_registry(1);
        let index = cache.entry(endpoint());
        for _ in 0..i32::from(mod_api::MAX_TERRAIN_EXTENT) / SLAB_HEIGHT {
            let query = Cache::slab_query(&cache.entries[index]);
            cache.ingest(index, &sample(query));
        }
        cache.entries[index].query
    }

    #[test]
    fn publishes_complete_xzy_snapshot_and_keeps_runtime_states() {
        let mut cache = Cache::default();
        cache.observe_world(&world());
        cache.observe_registry(1);
        let index = cache.entry(endpoint());
        let query = cache.entries[index].query;
        let slabs = i32::from(mod_api::MAX_TERRAIN_EXTENT) / SLAB_HEIGHT;
        for slab in 0..slabs {
            cache.ingest(index, &sample(Cache::slab_query(&cache.entries[index])));
            assert_eq!(cache.read(query).is_some(), slab == slabs - 1);
        }
        let region = cache.read(query).unwrap();
        let [x, y, z] = [3 - query.min.x, 64 - query.min.y, 4 - query.min.z].map(|v| v as usize);
        let cell = (x * usize::from(query.size.2) + z) * usize::from(query.size.1) + y;
        assert_eq!(region.palette[region.cells[cell] as usize].runtime_id, 2);
        let revision = region.stamp.revision;
        populate(&mut cache);
        assert_eq!(cache.read(query).unwrap().stamp.revision, revision);
    }

    #[test]
    fn unknown_cells_missing_layers_and_destroyed_openings_never_become_air() {
        for failure in 0..4 {
            let mut cache = Cache::default();
            let query = populate(&mut cache);
            let mut sample = sample(Cache::slab_query(&cache.entries[0]));
            match failure {
                0 => sample.loaded[0] = 0,
                1 => sample.states[0] = UNKNOWN,
                2 => sample.layer_count = 0,
                _ => sample.palette[0].identifier = None,
            }
            cache.ingest(0, &sample);
            assert!(cache.read(query).is_none());
        }
        let mut cache = Cache::default();
        let query = populate(&mut cache);
        for _ in 0..i32::from(mod_api::MAX_TERRAIN_EXTENT) / SLAB_HEIGHT {
            let mut sample = sample(Cache::slab_query(&cache.entries[0]));
            sample.states.fill(0);
            cache.ingest(0, &sample);
        }
        assert!(cache.read(query).is_none());
    }

    #[test]
    fn snapshots_survive_dimension_changes_but_not_world_connection_death_or_registry_changes() {
        for failure in 0..5 {
            let mut cache = Cache::default();
            let query = populate(&mut cache);
            let mut changed = world();
            changed.dimension = 0;
            cache.observe_world(&changed);
            assert!(cache.read(query).is_some());
            match failure {
                0 => changed.session += 1,
                1 => changed.world = "other".into(),
                2 => changed.alive = false,
                3 => {
                    cache.observe_registry(2);
                }
                _ => cache.clear(),
            }
            cache.observe_world(&changed);
            assert!(cache.read(query).is_none());
        }
    }

    #[test]
    fn cache_evicts_oldest_entries_at_its_fixed_capacity() {
        let mut cache = Cache::default();
        let query = populate(&mut cache);
        for i in 1..=MAX_SNAPSHOTS {
            let mut endpoint = endpoint();
            endpoint.portal.min[0] += i as i32 * 32;
            endpoint.portal.max[0] += i as i32 * 32;
            cache.entry(endpoint);
        }
        assert_eq!(cache.entries.len(), MAX_SNAPSHOTS);
        assert!(cache.read(query).is_none());
    }
}
