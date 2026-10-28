//! Retained visited-terrain snapshots with staged GPU resource replacement.
use crate::cache as api;
use crate::{
    live::{self, Endpoint},
    scenery::Inputs,
    view::{Resources, View},
};
use mod_api::drawing::{self, api::*};
use std::rc::Rc;

const FRAME_CELLS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Ready,
    Building,
    Unavailable,
    Unloaded,
    LostOpening,
}

struct Prepared {
    view: Option<View>,
    resources: Option<Resources>,
    palette: Vec<api::BlockState>,
    cells: Vec<u32>,
    side: i8,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        self.view = None;
        if let Some(resources) = self.resources.take() {
            resources.release();
        }
    }
}

pub struct Scene {
    query: api::Query,
    stamp: Option<api::Stamp>,
    active: Option<Prepared>,
    pending: Option<Prepared>,
    packing: Option<Packing>,
    status: Status,
}

impl Scene {
    pub fn new(destination: Endpoint) -> Self {
        Self {
            query: live::query(destination),
            stamp: None,
            active: None,
            pending: None,
            packing: None,
            status: Status::Building,
        }
    }

    pub fn hide(&mut self) {
        if let Some(view) = self.active.as_mut().and_then(|value| value.view.as_mut()) {
            view.hide();
        }
    }

    pub fn advance(
        &mut self,
        source: Endpoint,
        destination: Endpoint,
        side: i8,
        inputs: &Inputs,
        light: u8,
        cache: &api::Cache,
    ) -> Result<Status, DrawError> {
        if self.active.as_ref().is_some_and(|value| value.side != side)
            || self
                .packing
                .as_ref()
                .is_some_and(|value| value.side != side)
            || self
                .pending
                .as_ref()
                .is_some_and(|value| value.side != side)
        {
            self.active = None;
            self.pending = None;
            self.packing = None;
            self.stamp = None;
        }
        if let Some(pending) = &mut self.pending {
            if matches!(
                geometry_status(pending.resources.as_ref().unwrap().grid)?,
                GeometryState::Ready
            ) {
                let mut prepared = self.pending.take().unwrap();
                prepared.view.as_mut().unwrap().hide();
                prepared.view.as_mut().unwrap().show(true)?;
                self.active = Some(prepared);
                self.status = Status::Ready;
            } else {
                return Ok(Status::Building);
            }
        }
        let Some(region) = cache.read(self.query) else {
            self.hide();
            self.active = None;
            self.pending = None;
            self.packing = None;
            self.stamp = None;
            self.status = Status::Unavailable;
            return Ok(self.status);
        };
        if self.packing.is_some() {
            if !self.packing.as_mut().unwrap().step(FRAME_CELLS) {
                return Ok(Status::Building);
            }
            let packed = self.packing.take().unwrap();
            self.submit(
                source,
                destination,
                side,
                inputs,
                packed.grid,
                packed.region.palette.clone(),
            )?;
            return Ok(self.status);
        }
        if self.stamp != Some(region.stamp) {
            self.stamp = Some(region.stamp);
            if region.cells.iter().any(|cell| *cell == api::UNKNOWN) {
                self.hide();
                self.status = Status::Unloaded;
            } else if !live::destination_valid(destination, &self.query, &region) {
                self.hide();
                self.status = Status::LostOpening;
            } else {
                self.packing = Some(Packing::new(
                    self.query,
                    region,
                    source,
                    destination,
                    side,
                    light,
                ));
                self.status = Status::Building;
            }
        }
        if matches!(self.status, Status::Ready | Status::Building)
            && let Some(active) = &mut self.active
        {
            active.view.as_mut().unwrap().show(true)?;
        }
        Ok(self.status)
    }

    fn submit(
        &mut self,
        source: Endpoint,
        destination: Endpoint,
        side: i8,
        inputs: &Inputs,
        grid: BlockGrid,
        palette: Vec<api::BlockState>,
    ) -> Result<(), DrawError> {
        if let Some(active) = &mut self.active
            && active.palette.len() == palette.len()
            && active.palette.iter().zip(&palette).all(|(a, b)| a == b)
            && matches!(
                geometry_status(active.resources.as_ref().unwrap().grid)?,
                GeometryState::Ready
            )
        {
            let sy = u32::from(self.query.size.1);
            let sz = u32::from(self.query.size.2);
            let edits: Vec<_> = grid
                .cells
                .iter()
                .zip(&active.cells)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(FRAME_CELLS + 1)
                .map(|(index, (word, _))| {
                    let index = index as u32;
                    BlockEdit {
                        position: (index / (sz * sy), index % sy, (index / sy) % sz),
                        cell: drawing::unpack_block_cell(*word).unwrap(),
                    }
                })
                .collect();
            if edits.len() <= FRAME_CELLS {
                if !edits.is_empty() {
                    update_blocks(active.resources.as_ref().unwrap().grid, &edits)?;
                }
                active.cells = grid.cells;
                active.view.as_mut().unwrap().show(true)?;
                self.status = Status::Ready;
                return Ok(());
            }
        }
        let resources = Resources::upload_live(inputs, &grid)?;
        let mut view = match View::new_live(source, destination, &self.query, side, &resources) {
            Ok(view) => view,
            Err(error) => {
                resources.release();
                return Err(error);
            }
        };
        if let Err(error) = view.stage() {
            drop(view);
            resources.release();
            return Err(error);
        }
        self.pending = Some(Prepared {
            view: Some(view),
            resources: Some(resources),
            palette,
            cells: grid.cells,
            side,
        });
        self.status = Status::Building;
        Ok(())
    }
}

struct Packing {
    query: api::Query,
    region: Rc<api::Region>,
    grid: BlockGrid,
    words: Vec<u32>,
    normal: usize,
    direction: f64,
    center: f64,
    air: u32,
    side: i8,
}

impl Packing {
    fn new(
        query: api::Query,
        region: Rc<api::Region>,
        source: Endpoint,
        destination: Endpoint,
        side: i8,
        light: u8,
    ) -> Self {
        let normal = if destination.portal.axis == crate::portals::Axis::X {
            2
        } else {
            0
        };
        let direction = f64::from(side * source.side * destination.side);
        let center = destination.portal.center64()[normal];
        let mut palette: Vec<_> = region
            .palette
            .iter()
            .map(|state| BlockState::RuntimeId(state.runtime_id))
            .collect();
        let air = region
            .palette
            .iter()
            .position(|state| state.name == crate::portals::PORTAL_ID)
            .or_else(|| {
                region
                    .palette
                    .iter()
                    .position(|state| state.name == "minecraft:air")
            })
            .unwrap_or(palette.len()) as u32;
        let empty = BlockState::Named(NamedBlock {
            name: "minecraft:air".into(),
            states: "{}".into(),
        });
        if (air as usize) < palette.len() {
            palette[air as usize] = empty;
        } else {
            palette.push(empty);
        }
        let words: Vec<_> = (0..palette.len())
            .map(|index| {
                drawing::pack_block_cell(BlockCell {
                    palette: index as u32,
                    sky_light: Some(if destination.dimension == 0 { light } else { 0 }),
                    block_light: Some(light),
                })
                .unwrap()
            })
            .collect();
        let grid = BlockGrid {
            size: (
                u32::from(query.size.0),
                u32::from(query.size.1),
                u32::from(query.size.2),
            ),
            palette,
            cells: Vec::with_capacity(region.cells.len()),
        };
        Self {
            query,
            region,
            grid,
            words,
            normal,
            direction,
            center,
            air,
            side,
        }
    }

    /// Packs bounded work while keeping the prior complete geometry untouched.
    fn step(&mut self, budget: usize) -> bool {
        let sy = usize::from(self.query.size.1);
        let sz = usize::from(self.query.size.2);
        let end = self
            .grid
            .cells
            .len()
            .saturating_add(budget)
            .min(self.region.cells.len());
        for index in self.grid.cells.len()..end {
            let cell = self.region.cells[index];
            let offset = if self.normal == 0 {
                index / (sz * sy)
            } else {
                (index / sy) % sz
            };
            let coordinate = if self.normal == 0 {
                self.query.min.x
            } else {
                self.query.min.z
            };
            let beyond =
                (f64::from(coordinate) + offset as f64 + 0.5 - self.center) * self.direction > 0.;
            let palette_index =
                if beyond && self.region.palette[cell as usize].name != crate::portals::PORTAL_ID {
                    u32::from(cell)
                } else {
                    self.air
                };
            self.grid.cells.push(self.words[palette_index as usize]);
        }
        self.grid.cells.len() == self.region.cells.len()
    }
}

#[cfg(test)]
fn grid(
    query: &api::Query,
    region: &api::Region,
    source: Endpoint,
    destination: Endpoint,
    side: i8,
    light: u8,
) -> BlockGrid {
    let mut packing = Packing::new(
        *query,
        Rc::new(region.clone()),
        source,
        destination,
        side,
        light,
    );
    assert!(packing.step(usize::MAX));
    packing.grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portals::{Axis, Portal};

    #[test]
    fn sampled_cells_keep_xzy_order_and_only_the_far_half_is_drawn() {
        let endpoint = Endpoint {
            dimension: 1,
            portal: Portal {
                min: [0, 1, 0],
                max: [2, 4, 1],
                axis: Axis::X,
            },
            side: 1,
        };
        let query = api::Query {
            dimension: 1,
            min: api::Position { x: 0, y: 0, z: -1 },
            size: (2, 2, 4),
        };
        let region = api::Region {
            stamp: api::Stamp {
                session: 1,
                revision: 1,
            },
            palette: vec![api::BlockState {
                runtime_id: 7,
                name: "minecraft:netherrack".into(),
                states: "{}".into(),
            }],
            cells: vec![0; 16],
        };
        let grid = grid(&query, &region, endpoint, endpoint, 1, 10);
        for x in 0..2 {
            for z in 0..4 {
                for y in 0..2 {
                    let word = grid.cells[(x * 4 + z) * 2 + y];
                    assert_eq!(
                        drawing::unpack_block_cell(word).unwrap().palette,
                        if z >= 2 { 0 } else { 1 }
                    );
                }
            }
        }
    }
}
