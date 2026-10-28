use crate::{
    config::Config,
    portals::{self, Portal, VISIBLE_PORTALS},
    scenery::Inputs,
    view::{self, Resources, View},
};
use mod_api::{
    bindings::cinnabar::extension::{
        budget, gameplay, hud, player_state, settings,
        terrain::{self, ReadResult, Stamp},
    },
    drawing::api::*,
};

/// These IDs belong to separate namespaces. Each is compared only against its own prior value.
#[derive(PartialEq)]
struct Scope {
    actor: u64,
    connection: u64,
    dimension: i32,
    world: String,
}

#[derive(Default)]
struct Slot {
    portal: Option<Portal>,
    stamp: Option<Stamp>,
    valid: bool,
    view: Option<View>,
    live: Option<crate::live_scene::Scene>,
}

enum Phase {
    Idle,
    Building,
    Ready,
    Live(crate::live_scene::Status),
    Unlinked,
}

pub struct Runtime {
    config: Config,
    usable: bool,
    scope: Option<Scope>,
    inputs: Option<Inputs>,
    resources: Option<Resources>,
    identifiers: [String; 1],
    points: Vec<[i32; 3]>,
    candidates: Vec<Portal>,
    detector: Option<portals::Detector>,
    slots: [Slot; VISIBLE_PORTALS],
    failed: Option<String>,
    retries: u8,
    status: String,
    links: crate::live::Links,
    live_cursor: usize,
    cache: crate::cache::Cache,
    capture_cursor: usize,
    capture_cooldown: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            config: Config::default(),
            usable: false,
            scope: None,
            inputs: None,
            resources: None,
            identifiers: [portals::PORTAL_ID.into()],
            points: Vec::new(),
            candidates: Vec::new(),
            detector: None,
            slots: [Slot::default(), Slot::default()],
            failed: None,
            retries: 0,
            status: String::new(),
            links: Default::default(),
            live_cursor: 0,
            cache: Default::default(),
            capture_cursor: 0,
            capture_cooldown: false,
        }
    }
}

fn usable_frame(frame: &gameplay::Snapshot, world: &player_state::WorldObservation) -> bool {
    world.alive
        && frame.dimension == world.dimension
        && [frame.eye.x, frame.eye.y, frame.eye.z]
            .iter()
            .all(|v| v.is_finite())
}

impl Runtime {
    pub fn init(&mut self) {
        let config = settings::load().map_or(Ok(Config::default()), |json| Config::parse(&json));
        match config {
            Ok(config) => {
                self.config = config;
                self.usable = self.config.enabled;
                if self.usable {
                    self.inputs = Some(Inputs::new(&self.config));
                }
            }
            Err(error) => self.status(&format!("Portal settings: {error}")),
        }
    }

    fn status(&mut self, message: &str) {
        if self.config.show_status && self.status != message {
            let message: String = message.chars().take(220).collect();
            let _ = hud::set_label(&message);
            self.status = message;
        }
    }

    fn clear_rendering(&mut self) {
        for slot in &mut self.slots {
            slot.view = None;
            slot.live = None;
        }
        if let Some(resources) = self.resources.take() {
            resources.release();
        }
    }

    fn reset(&mut self) {
        self.clear_rendering();
        self.slots = [Slot::default(), Slot::default()];
        self.points.clear();
        self.candidates.clear();
        self.detector = None;
        self.failed = None;
        self.retries = 0;
    }

    pub fn frame(&mut self) {
        if !self.usable {
            return;
        }
        let frame = gameplay::read_frame().ok().flatten();
        let world = player_state::read_world().ok().flatten();
        if let Some(world) = &world {
            self.cache.observe_world(world);
        }
        if world.as_ref().is_some_and(|world| !world.alive) {
            self.links.clear();
        }
        // A pause/container releases gameplay input without invalidating world-owned geometry.
        if frame.is_none()
            && world.as_ref().is_some_and(|world| {
                world.alive
                    && self.scope.as_ref().is_some_and(|scope| {
                        scope.connection == world.session
                            && scope.dimension == world.dimension
                            && scope.world == world.world
                    })
            })
        {
            return;
        }
        let Some((frame, world)) = frame.zip(world).filter(|(f, w)| usable_frame(f, w)) else {
            self.reset();
            self.scope = None;
            self.status("");
            return;
        };
        self.links.observe_world(&world);
        let unchanged = self.scope.as_ref().is_some_and(|scope| {
            scope.actor == frame.session
                && scope.connection == world.session
                && scope.dimension == world.dimension
                && scope.world == world.world
        });
        if !unchanged {
            self.reset();
            self.scope = Some(Scope {
                actor: frame.session,
                connection: world.session,
                dimension: world.dimension,
                world: world.world.clone(),
            });
        }
        if let Some(error) = self.failed.clone() {
            self.status(&error);
            return;
        }
        let eye = gameplay::read_camera()
            .ok()
            .flatten()
            .map(|camera| [camera.position.x, camera.position.y, camera.position.z])
            .filter(|eye| eye.iter().all(|v| v.is_finite()))
            .unwrap_or([frame.eye.x, frame.eye.y, frame.eye.z]);
        match self.advance(&world, eye) {
            Ok(Phase::Building) => self.status("Portal scenes: preparing native terrain..."),
            Ok(Phase::Ready) => {
                self.retries = 0;
                self.status("");
            }
            Ok(Phase::Idle) => self.status(""),
            Ok(Phase::Unlinked) => {
                self.status("Cached portal: cross normally, step out and wait a moment, then return to learn this direction.")
            }
            Ok(Phase::Live(status)) => {
                if status == crate::live_scene::Status::Ready {
                    self.retries = 0;
                }
                self.status(match status {
                crate::live_scene::Status::Ready => "Cached portal: blocks from your last visit; changes elsewhere are not live.",
                crate::live_scene::Status::Building => "Cached portal: preparing visited terrain...",
                crate::live_scene::Status::Unavailable => "Cached portal: no complete destination snapshot. Revisit the other side and wait near its opening.",
                crate::live_scene::Status::Unloaded => "Cached portal: destination snapshot incomplete; preview hidden.",
                crate::live_scene::Status::LostOpening => "Cached portal: observed destination opening changed; cross again to relearn.",
            });
            }
            Err(error) => {
                self.clear_rendering();
                let message = format!("Portal scenes: {:?}: {}", error.code, error.detail);
                let retry = matches!(error.code, ErrorCode::StaleHandle)
                    || matches!(error.code, ErrorCode::Unsupported) && self.retries == 0;
                if retry {
                    self.retries += 1;
                } else {
                    self.failed = Some(message.clone());
                }
                self.status(&message);
            }
        }
    }

    fn advance(
        &mut self,
        world: &player_state::WorldObservation,
        eye: [f32; 3],
    ) -> Result<Phase, DrawError> {
        let Ok(found) = terrain::find_blocks(
            &self.identifiers,
            self.config.search_distance,
            u32::from(mod_api::MAX_TERRAIN_EXTENT).pow(2),
        ) else {
            self.reset();
            return Ok(Phase::Idle);
        };
        let mut points: Vec<_> = found.into_iter().map(|p| [p.x, p.y, p.z]).collect();
        points.sort_unstable();
        if points != self.points {
            self.points = points;
            self.detector = Some(portals::Detector::new(&self.points));
        }
        if budget::remaining_fuel() < 80_000 {
            return Ok(Phase::Building);
        }
        if let Some(detector) = &mut self.detector
            && let Some(candidates) = detector.step(24)
        {
            self.candidates = candidates;
            self.detector = None;
        }
        self.candidates.sort_by(|a, b| {
            a.distance_squared(eye)
                .total_cmp(&b.distance_squared(eye))
                .then_with(|| a.min.cmp(&b.min))
        });
        let mut previous = std::mem::replace(&mut self.slots, [Slot::default(), Slot::default()]);
        let mut registry_changed = false;
        for (index, portal) in self
            .candidates
            .iter()
            .copied()
            .take(self.config.max_portals)
            .enumerate()
        {
            if budget::remaining_fuel() < 50_000 {
                break;
            }
            let mut slot = previous
                .iter_mut()
                .find(|slot| slot.portal == Some(portal))
                .map(std::mem::take)
                .unwrap_or(Slot {
                    portal: Some(portal),
                    ..Default::default()
                });
            match terrain::read_region(portal.query(), slot.stamp) {
                Ok(ReadResult::Changed(region))
                    if region.stamp.session == world.session
                        && region.stamp.dimension == world.dimension =>
                {
                    registry_changed |= self.cache.observe_registry(region.stamp.registry);
                    slot.valid = portals::verify(portal, &region);
                    slot.stamp = Some(region.stamp);
                }
                Ok(ReadResult::Unchanged(stamp))
                    if slot.stamp.as_ref().is_some_and(|old| {
                        old.session == stamp.session
                            && old.dimension == stamp.dimension
                            && old.registry == stamp.registry
                            && old.revision == stamp.revision
                    }) => {}
                _ => {
                    slot.valid = false;
                    slot.stamp = None;
                }
            }
            if !slot.valid {
                slot.view = None;
                slot.live = None;
            }
            self.slots[index] = slot;
        }
        drop(previous);
        if registry_changed {
            for slot in &mut self.slots {
                slot.live = None;
            }
        }
        if !self.slots.iter().any(|slot| slot.valid) {
            return Ok(Phase::Idle);
        }
        let valid: Vec<_> = self
            .slots
            .iter()
            .filter(|slot| slot.valid)
            .filter_map(|slot| slot.portal)
            .collect();
        let feet = [world.position.0, world.position.1, world.position.2];
        // Unselected candidates still make a crossing ambiguous; the view count is not a link filter.
        if self
            .candidates
            .iter()
            .filter(|portal| crate::live::near(**portal, feet, 3.))
            .count()
            == 1
        {
            self.links.observe_portals(world.dimension, feet, &valid);
        }
        if self.config.cached_terrain {
            // Search + at most two opening checks leave one of the four terrain queries for capture.
            let portal = valid[self.capture_cursor % valid.len()];
            // Keep a pending query subscribed until its asynchronous reply can be consumed.
            if self.capture_cooldown {
                // Leave a frame for packing/uploading previews after each consumed slab.
                self.capture_cooldown = false;
            } else if budget::remaining_fuel() >= 100_000 && self.cache.capture(world, portal) {
                self.capture_cursor = self.capture_cursor.wrapping_add(1);
                self.capture_cooldown = true;
            }
            let mut status = None;
            let start = self.live_cursor % self.slots.len();
            self.live_cursor = (start + 1) % self.slots.len();
            for offset in 0..self.slots.len() {
                let index = (start + offset) % self.slots.len();
                let slot = &mut self.slots[index];
                if !slot.valid {
                    continue;
                }
                if budget::remaining_fuel() < 120_000 {
                    status.get_or_insert(crate::live_scene::Status::Building);
                    continue;
                }
                let portal = slot.portal.unwrap();
                let Some((source, destination)) = self.links.destination(world.dimension, portal)
                else {
                    continue;
                };
                let (side, distance) = view::side_distance(portal, eye);
                if distance < 0.35 {
                    if let Some(scene) = &mut slot.live {
                        scene.hide();
                    }
                    continue;
                }
                let scene = slot
                    .live
                    .get_or_insert_with(|| crate::live_scene::Scene::new(destination));
                let result = scene.advance(
                    source,
                    destination,
                    side,
                    self.inputs.as_ref().unwrap(),
                    self.config.ambient_light,
                    &self.cache,
                )?;
                if result == crate::live_scene::Status::LostOpening {
                    self.links.invalidate(world.dimension, portal);
                    slot.live = None;
                }
                status = Some(result);
            }
            return Ok(status.map_or(Phase::Unlinked, Phase::Live));
        }
        if self.resources.is_none() {
            self.resources = Some(Resources::upload(
                self.inputs.as_ref().expect("enabled inputs"),
                world.dimension,
            )?);
        }
        let resources = self.resources.as_ref().unwrap();
        let ready = matches!(geometry_status(resources.grid)?, GeometryState::Ready);
        let mut building = false;
        let mut visible = false;
        for slot in &mut self.slots {
            if !slot.valid {
                continue;
            }
            let portal = slot.portal.unwrap();
            let (side, distance) = view::side_distance(portal, eye);
            if distance < 0.35 {
                if let Some(view) = &mut slot.view {
                    view.hide();
                }
                continue;
            }
            if slot.view.as_ref().is_some_and(|view| view.side != side) {
                slot.view = None;
            }
            if slot.view.is_none() {
                slot.view = Some(View::new(portal, side, world.dimension, resources)?);
            }
            let view = slot.view.as_mut().unwrap();
            view.show(ready)?;
            building |= !ready || !matches!(clip_status(view.clip)?, GeometryState::Ready);
            visible = true;
        }
        Ok(if building {
            Phase::Building
        } else if visible {
            Phase::Ready
        } else {
            Phase::Idle
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_session_namespaces_and_life_guards_are_preserved() {
        let mut frame = gameplay::Snapshot {
            session: 17,
            dimension: 0,
            eye: gameplay::Vector3 {
                x: 1.,
                y: 65.6,
                z: 6.,
            },
            yaw: 0.,
            pitch: 0.,
            frame_seconds: 0.016,
            players: vec![],
        };
        let mut world = player_state::WorldObservation {
            session: 2,
            world: "fixture".into(),
            dimension: 0,
            position: (1., 64., 6.),
            alive: true,
            runtime_id: 1,
        };
        assert!(usable_frame(&frame, &world));
        world.alive = false;
        assert!(!usable_frame(&frame, &world));
        world.alive = true;
        world.dimension = 1;
        assert!(!usable_frame(&frame, &world));
        world.dimension = 0;
        frame.eye.x = f32::NAN;
        assert!(!usable_frame(&frame, &world));
    }
}
