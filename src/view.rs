//! Retained native block geometry and camera-side portal placements.
use crate::{
    portals::{Axis, Portal},
    scenery::{self, Inputs},
};
use mod_api::drawing::{self, api::*};

pub struct Resources {
    pub grid: Handle,
    plane: Handle,
    sky: Handle,
    material: Handle,
    pub size: [u32; 3],
}

impl Resources {
    pub fn upload_live(inputs: &Inputs, grid: &BlockGrid) -> Result<Self, DrawError> {
        Self::upload_grid(
            inputs,
            &GeometrySpec::Blocks(grid.clone()),
            [grid.size.0, grid.size.1, grid.size.2],
        )
    }

    pub fn upload(inputs: &Inputs, dimension: i32) -> Result<Self, DrawError> {
        Self::upload_grid(
            inputs,
            if dimension == 1 {
                &inputs.overworld
            } else {
                &inputs.nether
            },
            inputs.size,
        )
    }

    fn upload_grid(
        inputs: &Inputs,
        spec: &GeometrySpec,
        size: [u32; 3],
    ) -> Result<Self, DrawError> {
        let mut created = Vec::new();
        let result = (|| {
            let plane = create_geometry(&inputs.plane)?;
            created.push(plane);
            let sky = create_geometry(&inputs.sky)?;
            created.push(sky);
            let material = create_material(scenery::SKY_SHADER)?;
            created.push(material);
            let grid = create_geometry(spec)?;
            created.push(grid);
            Ok(Self {
                grid,
                plane,
                sky,
                material,
                size,
            })
        })();
        if result.is_err() {
            for id in created.into_iter().rev() {
                let _ = remove(id);
            }
        }
        result
    }

    pub fn release(self) {
        for id in [self.grid, self.sky, self.plane, self.material] {
            let _ = remove(id);
        }
    }
}

pub struct View {
    pub side: i8,
    pub clip: Handle,
    grid: Instance,
    sky: Instance,
    pub grid_draw: Option<Handle>,
    sky_draw: Option<Handle>,
}

/// Rotation takes local +Z toward the viewer; the local -Z half-space holds the scenery.
pub fn orientation(axis: Axis, side: i8) -> (f32, f32) {
    match (axis, side > 0) {
        (Axis::X, true) => (0., 1.),
        (Axis::X, false) => (0., -1.),
        (Axis::Z, true) => (1., 0.),
        (Axis::Z, false) => (-1., 0.),
    }
}

pub fn side_distance(portal: Portal, eye: [f32; 3]) -> (i8, f32) {
    let normal = if portal.axis == Axis::X { 2 } else { 0 };
    let distance = eye[normal] - portal.center()[normal];
    (if distance >= 0. { 1 } else { -1 }, distance.abs())
}

fn rotate(v: [f32; 3], s: f32, c: f32) -> (f32, f32, f32) {
    (v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c)
}

impl View {
    pub fn new_live(
        source: crate::live::Endpoint,
        destination: crate::live::Endpoint,
        query: &crate::cache::Query,
        side: i8,
        resources: &Resources,
    ) -> Result<Self, DrawError> {
        let mut view = Self::new(source.portal, side, source.dimension, resources)?;
        let (s, c) = crate::live::rotation(source, destination);
        let angle = s.atan2(c);
        view.grid.transform.rotation = (0., (angle * 0.5).sin(), 0., (angle * 0.5).cos());
        let center = destination.portal.center64();
        view.grid.transform.translation = rotate(
            [
                (f64::from(query.min.x) - center[0]) as f32,
                (f64::from(query.min.y) - center[1]) as f32,
                (f64::from(query.min.z) - center[2]) as f32,
            ],
            s,
            c,
        );
        view.sky.params = vec![if destination.dimension == 0 { 1. } else { 0. }];
        Ok(view)
    }

    /// Starts upload outside the view while the previous complete scene remains visible.
    pub fn stage(&mut self) -> Result<(), DrawError> {
        let mut hidden = self.grid.clone();
        if let Anchor::World(ref mut anchor) = hidden.anchor {
            anchor.origin.1 -= 1_000_000.;
        }
        self.grid_draw = Some(create(&hidden)?);
        Ok(())
    }

    pub fn new(
        portal: Portal,
        side: i8,
        dimension: i32,
        resources: &Resources,
    ) -> Result<Self, DrawError> {
        let (s, c) = orientation(portal.axis, side);
        let angle = s.atan2(c);
        let rotation = (0., (angle * 0.5).sin(), 0., (angle * 0.5).cos());
        let anchor = Anchor::World(WorldAnchor {
            dimension,
            origin: (
                (f64::from(portal.min[0]) + f64::from(portal.max[0])) * 0.5,
                (f64::from(portal.min[1]) + f64::from(portal.max[1])) * 0.5,
                (f64::from(portal.min[2]) + f64::from(portal.max[2])) * 0.5,
            ),
        });
        let width = (portal.max[portal.width_axis()] - portal.min[portal.width_axis()]) as f32;
        let height = (portal.max[1] - portal.min[1]) as f32;
        let clip = create_clip(ClipSpec {
            geometry: resources.plane,
            anchor,
            transform: Transform {
                translation: (s * 0.127, 0., c * 0.127),
                rotation,
                scale: (width, height, 1.),
            },
            parent: None,
        })?;
        let mut grid = drawing::instance(resources.grid, anchor);
        grid.clip = Some(clip);
        grid.transform.rotation = rotation;
        let [w, h, d] = resources.size.map(|v| v as f32);
        grid.transform.translation = rotate(
            [-w * 0.5, -(scenery::FLOOR as f32) - height * 0.5, -d],
            s,
            c,
        );
        let mut sky = drawing::instance(resources.sky, anchor);
        sky.clip = Some(clip);
        sky.material = Some(resources.material);
        sky.params = vec![if dimension == 1 { 1. } else { 0. }];
        sky.transform.rotation = rotation;
        // Ordinary triangle geometry has a smaller local extent budget than block grids.
        // Put the backdrop's offset in its double-precision world anchor, keeping its local bounds small.
        sky.transform.scale = (w + 2., h + 8., d + 2.);
        let offset = rotate(
            [
                0.,
                h * 0.5 - (scenery::FLOOR as f32) - height * 0.5,
                -(d + 2.) * 0.5,
            ],
            s,
            c,
        );
        let Anchor::World(origin) = anchor else {
            unreachable!()
        };
        sky.anchor = Anchor::World(WorldAnchor {
            dimension,
            origin: (
                origin.origin.0 + f64::from(offset.0),
                origin.origin.1 + f64::from(offset.1),
                origin.origin.2 + f64::from(offset.2),
            ),
        });
        Ok(Self {
            side,
            clip,
            grid,
            sky,
            grid_draw: None,
            sky_draw: None,
        })
    }

    /// A grid placement starts the asynchronous native upload. Air background waits for readiness.
    pub fn show(&mut self, ready: bool) -> Result<(), DrawError> {
        if self.grid_draw.is_none() {
            self.grid_draw = Some(create(&self.grid)?);
        }
        if ready && self.sky_draw.is_none() {
            self.sky_draw = Some(create(&self.sky)?);
        }
        Ok(())
    }

    /// Retains the mask so walking near its plane does not churn clip resources every frame.
    pub fn hide(&mut self) {
        if let Some(id) = self.sky_draw.take() {
            let _ = remove(id);
        }
        if let Some(id) = self.grid_draw.take() {
            let _ = remove(id);
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.hide();
        let _ = remove(self.clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_axes_and_sides_put_the_room_behind_the_window() {
        for axis in [Axis::X, Axis::Z] {
            for side in [-1, 1] {
                let (s, c) = orientation(axis, side);
                let behind = rotate([0., 0., -10.], s, c);
                let normal = if axis == Axis::X { behind.2 } else { behind.0 };
                assert!(normal * f32::from(side) < 0.);
                let plane = rotate([0., 0., 0.127], s, c);
                let front = if axis == Axis::X { plane.2 } else { plane.0 };
                assert!(front * f32::from(side) > 0.);
            }
        }
    }
}
