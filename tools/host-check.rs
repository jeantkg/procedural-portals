//! Actual packed guest + native mesher. GPU completion alone is simulated without a client.
use mod_host::{
    BlockSearch, BlockSearchSource, FoundBlocks, GameplaySnapshot, GameplayVector3, ModGrants,
    ModHost, TerrainBlockState, TerrainCheck, TerrainPosition, TerrainQuery, TerrainReadError,
    TerrainRefresh, TerrainRegion, TerrainScheduler, TerrainShapeBinding, TerrainSource,
    TerrainStamp, WorldObservation,
};
use std::{collections::HashMap, sync::Arc};

struct Native {
    assets: Arc<assets::RuntimeAssets>,
    ids: HashMap<(String, String), u32>,
    triangles: usize,
    grids: usize,
}
impl Native {
    fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let assets = Arc::new(assets::RuntimeAssets::decode(&std::fs::read(path)?)?);
        let target: serde_json::Value =
            serde_json::from_slice(&std::fs::read("assets/bedrock-target.json")?)?;
        let records = assets::read_registry_for_protocol(
            &std::fs::read(target["artifacts"]["block_registry"].as_str().unwrap())?,
            target["wire_protocol"].as_u64().unwrap() as u32,
        )?;
        let mut ids = HashMap::new();
        for record in records.iter() {
            let state: serde_json::Value = serde_json::from_str(&record.canonical_state)?;
            ids.insert(
                (record.name.to_string(), state.to_string()),
                record.sequential_id,
            );
        }
        Ok(Self {
            assets,
            ids,
            triangles: 0,
            grids: 0,
        })
    }
    fn complete(&mut self, host: &ModHost) -> anyhow::Result<()> {
        for resources in &host.render().0.retained {
            for geometry in resources.geometry.values() {
                if let Some(block) = geometry.blocks()
                    && block.claim()
                {
                    let mesh = block
                        .mesh(
                            &self.assets,
                            assets::NetworkIdMode::Sequential,
                            |name, states| {
                                let state: serde_json::Value = serde_json::from_str(states).ok()?;
                                self.ids.get(&(name.to_owned(), state.to_string())).copied()
                            },
                            Default::default(),
                        )
                        .map_err(|error| {
                            anyhow::anyhow!(
                                "native palette/meshing: {:?}: {}",
                                error.code,
                                error.detail
                            )
                        })?;
                    anyhow::ensure!(mesh.triangles > 0, "empty native scene mesh");
                    self.triangles += mesh.triangles;
                    self.grids += 1;
                    block.complete(Ok(mesh));
                    block.complete_render();
                }
            }
            for clip in resources.clips.values() {
                clip.status.complete(Ok(()));
            }
        }
        Ok(())
    }
}

struct Fixture {
    broken: bool,
    axis_z: bool,
    dimension: i32,
    eye: [f32; 3],
    peak: u64,
    native: Native,
    revision: u64,
    unknown: bool,
    connection: u64,
    registry: u64,
    ticks: usize,
}
impl Fixture {
    fn world(&self) -> WorldObservation {
        WorldObservation {
            session: self.connection,
            world: "fixture".into(),
            dimension: self.dimension,
            position: (self.eye[0], self.eye[1] - 1.6, self.eye[2]),
            alive: true,
            runtime_id: 1,
        }
    }
}
impl BlockSearchSource for Fixture {
    fn find(&mut self, _: &BlockSearch) -> Result<FoundBlocks, TerrainReadError> {
        let axis_z = self.axis_z;
        Ok(FoundBlocks {
            session: self.connection,
            dimension: self.dimension,
            positions: if self.broken {
                vec![]
            } else {
                [0, 1, 8, 9]
                    .into_iter()
                    .flat_map(|u| {
                        (64..67).map(move |y| TerrainPosition {
                            x: if axis_z { 0 } else { u },
                            y,
                            z: if axis_z { u } else { 0 },
                        })
                    })
                    .collect()
            },
        })
    }
}
impl TerrainSource for Fixture {
    fn check(&mut self, _: &TerrainQuery, retained: Option<&TerrainRegion>) -> TerrainCheck {
        if retained.is_some_and(|r| r.stamp.session == self.connection && r.stamp.dimension == self.dimension && r.stamp.revision == self.revision && r.stamp.registry == self.registry) {
            TerrainCheck::Current
        } else {
            TerrainCheck::Stale { cells: 20 }
        }
    }
    fn sample(&mut self, query: &TerrainQuery) -> TerrainRefresh {
        let mut states = Vec::new();
        for x in query.min.x..query.max.x {
            for z in query.min.z..query.max.z {
                for y in query.min.y..query.max.y {
                    let inside = if self.axis_z {
                        x == 0 && ((0..2).contains(&z) || (8..10).contains(&z))
                    } else {
                        z == 0 && ((0..2).contains(&x) || (8..10).contains(&x))
                    };
                    states.push(if !self.broken && inside && (64..67).contains(&y) { 1 }
                        else if y < 64 || x == 3 && z == 4 && y < 70 { 2 } else { 0 });
                }
            }
        }
        let loaded = vec![if self.unknown && states.len() >= 256 { 0 } else { u64::MAX }; states.len().div_ceil(64)];
        let block = |identifier: &str, canonical_state: Option<String>| TerrainBlockState {
            runtime_id: *self.native.ids.get(&(identifier.to_owned(),
                serde_json::from_str::<serde_json::Value>(canonical_state.as_deref().unwrap_or("{}")).unwrap().to_string())).expect("native fixture block"),
            identifier: Some(identifier.into()),
            canonical_state,
            tags: vec![],
            physics: None,
            collision: TerrainShapeBinding::NotRequested,
            selection: TerrainShapeBinding::NotRequested,
        };
        TerrainRefresh::Replace(Box::new(TerrainRegion {
            stamp: TerrainStamp {
                session: self.connection,
                dimension: self.dimension,
                registry: self.registry,
                revision: self.revision,
            },
            chunks: vec![],
            layer_count: 1,
            loaded,
            states,
            palette: vec![
                block("minecraft:air", None),
                block(
                    "minecraft:portal",
                    Some(format!(
                        r#"{{"portal_axis":{{"type":"string","value":"{}"}}}}"#,
                        if self.axis_z { "z" } else { "x" }
                    )),
                ),
                block(if self.revision == 0 { "minecraft:netherrack" } else { "minecraft:glowstone" }, None),
            ],
            shape_offsets: vec![0],
            shape_boxes: vec![],
            overrides: vec![],
        }))
    }
}

fn tick(
    host: &mut ModHost,
    fixture: &mut Fixture,
    scheduler: &mut TerrainScheduler,
) -> anyhow::Result<()> {
    scheduler.refresh(&mut [&mut *host], 8192, fixture);
    mod_host::refresh_block_searches(&mut [&mut *host], fixture);
    host.observe_presentation(Some(fixture.world()), None, None, vec![])?;
    fixture.ticks += 1;
    let result = host.frame_with_gameplay(
        false,
        Some(GameplaySnapshot {
            session: 17,
            dimension: fixture.dimension,
            eye: GameplayVector3 {
                x: fixture.eye[0],
                y: fixture.eye[1],
                z: fixture.eye[2],
            },
            yaw: 0.,
            pitch: 0.,
            attack_held: false,
            frame_seconds: 0.016,
            players: vec![],
        }),
    );
    if let Err(error) = result {
        return Err(anyhow::anyhow!("tick {} dimension {} previous peak {} HUD {:?}: {error:#}", fixture.ticks, fixture.dimension, fixture.peak, host.label()));
    }
    fixture.peak = fixture.peak.max(host.last_fuel_used());
    anyhow::ensure!(
        host.is_active() && !host.is_recovering(),
        "guest exceeded frame fuel or stopped"
    );
    fixture.native.complete(host)?;
    Ok(())
}

fn enabled(host: &ModHost) -> bool {
    host.render().0.retained.iter().any(|resources| {
        resources.instances.values().any(|instance| {
            instance.clip.is_some()
                && resources
                    .geometry
                    .get(&instance.geometry)
                    .is_some_and(|g| g.blocks().is_some())
        })
    })
}
fn geometry_ids(host: &ModHost) -> Vec<(u64, u64, u64)> {
    host.render()
        .0
        .retained
        .iter()
        .flat_map(|r| {
            r.geometry
                .iter()
                .filter(|(_, g)| g.blocks().is_some())
                .map(|(id, _)| (id.owner, id.generation, id.slot))
        })
        .collect()
}

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let component = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("supply component and native world carrier"))?;
    let carrier = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing native .mcbea carrier fixture"))?;
    let mut host = ModHost::load_with_grants(
        std::path::Path::new(&component),
        ModGrants {
            terrain: true,
            player_state: true,
            settings: true,
            world_draw: true,
            render_material: true,
            hud: true,
            ..Default::default()
        },
    )?;
    println!("Init fuel={}", host.last_fuel_used());
    let mut fixture = Fixture {
        broken: false,
        axis_z: false,
        dimension: 0,
        eye: [1., 65.6, 6.],
        peak: 0,
        native: Native::load(std::path::Path::new(&carrier))?,
        revision: 0,
        unknown: false,
        connection: 2,
        registry: 1,
        ticks: 0,
    };
    let mut scheduler = TerrainScheduler::default();
    let settings_path = std::path::Path::new(&component).with_extension("settings.json");
    let settings: serde_json::Value = std::fs::read(settings_path).ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    if settings.get("cached_terrain").and_then(|v| v.as_bool()).unwrap_or(true) {
        return check_cache(&mut host, &mut fixture, &mut scheduler);
    }
    for _ in 0..12 {
        tick(&mut host, &mut fixture, &mut scheduler)?;
    }
    anyhow::ensure!(
        enabled(&host),
        "portal did not activate: {:?}",
        host.label()
    );
    let settings_path = std::path::Path::new(&component).with_extension("settings.json");
    let settings: serde_json::Value = std::fs::read(settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let expected = settings
        .get("max_portals")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;
    let views = host
        .render()
        .0
        .retained
        .iter()
        .flat_map(|r| {
            r.instances.values().filter(|i| {
                r.geometry
                    .get(&i.geometry)
                    .is_some_and(|g| g.blocks().is_some())
            })
        })
        .count();
    anyhow::ensure!(
        views == expected,
        "expected {expected} block views, got {views}: {:?}",
        host.label()
    );
    anyhow::ensure!(
        host.render().0.passes.is_empty(),
        "legacy post shader remained active"
    );
    let geometry = geometry_ids(&host);
    host.observe_presentation(Some(fixture.world()), None, None, vec![])?;
    host.frame(false)?;
    anyhow::ensure!(
        enabled(&host) && geometry_ids(&host) == geometry,
        "pause rebuilt or removed scene geometry"
    );
    fixture.eye[2] = 0.5;
    tick(&mut host, &mut fixture, &mut scheduler)?;
    anyhow::ensure!(
        !enabled(&host),
        "contents must hide inside the native portal plane"
    );
    fixture.eye[2] = -6.;
    // Pause drops input-scoped terrain subscriptions; allow their asynchronous refresh on resume.
    for _ in 0..4 {
        tick(&mut host, &mut fixture, &mut scheduler)?;
    }
    anyhow::ensure!(
        enabled(&host) && geometry_ids(&host) == geometry,
        "backside lost cached geometry"
    );
    fixture.broken = true;
    for _ in 0..4 {
        tick(&mut host, &mut fixture, &mut scheduler)?;
    }
    anyhow::ensure!(!enabled(&host), "broken portal remained visible");
    fixture.broken = false;
    fixture.axis_z = true;
    fixture.eye = [6., 65.6, 1.];
    for _ in 0..12 {
        tick(&mut host, &mut fixture, &mut scheduler)?;
    }
    anyhow::ensure!(enabled(&host), "Z-axis portal failed: {:?}", host.label());
    host.observe_presentation(None, None, None, vec![])?;
    host.frame(false)?;
    anyhow::ensure!(!enabled(&host), "loading gap retained native objects");
    fixture.dimension = 1;
    for _ in 0..12 {
        tick(&mut host, &mut fixture, &mut scheduler)?;
    }
    anyhow::ensure!(
        enabled(&host),
        "Nether return view failed: {:?}",
        host.label()
    );
    println!(
        "PASS: actual WASM + native terrain mesher; activation, cache reuse, both sides/axes, portal break, pause, loading gap and dimension change"
    );
    println!(
        "Native grids={}, triangles={}, peak callback fuel={}. GPU admission was simulated; in-game rendering remains separate.",
        fixture.native.grids, fixture.native.triangles, fixture.peak
    );
    Ok(())
}

fn check_cache(host: &mut ModHost, fixture: &mut Fixture, scheduler: &mut TerrainScheduler) -> anyhow::Result<()> {
    fn settle(host: &mut ModHost, fixture: &mut Fixture, scheduler: &mut TerrainScheduler) -> anyhow::Result<()> {
        for _ in 0..240 { tick(host, fixture, scheduler)?; }
        Ok(())
    }
    fn visible_state(host: &ModHost, runtime_id: u32) -> bool {
        host.render().0.retained.iter().any(|r| r.instances.values().any(|i| {
            r.geometry.get(&i.geometry).and_then(|g| g.blocks()).is_some_and(|b| {
                b.grid.cells.iter().any(|cell| matches!(b.grid.palette[cell.palette as usize],
                    mod_render::retained::blocks::BlockState::RuntimeId(id) if id == runtime_id))
            })
        }))
    }
    fixture.eye = [1., 65.6, 1.5];
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(!enabled(host), "unobserved destination was invented");
    fixture.dimension = 1;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(!enabled(host), "reverse link was invented");
    fixture.dimension = 0;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(enabled(host) && host.label().is_some_and(|label| label.contains("last visit")), "cached destination did not activate: {:?}", host.label());
    let netherrack = fixture.native.ids[&("minecraft:netherrack".into(), "{}".into())];
    let glowstone = fixture.native.ids[&("minecraft:glowstone".into(), "{}".into())];
    anyhow::ensure!(visible_state(host, netherrack) && !visible_state(host, glowstone), "destination snapshot did not preserve native runtime states");
    let geometry = geometry_ids(host);
    fixture.revision = 1;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(geometry_ids(host) == geometry && visible_state(host, netherrack) && !visible_state(host, glowstone), "away-dimension changes leaked into the cached view");
    host.observe_presentation(Some(fixture.world()), None, None, vec![])?;
    host.frame(false)?;
    anyhow::ensure!(enabled(host) && geometry_ids(host) == geometry, "pause discarded a cached view");
    fixture.eye[2] = 0.5;
    tick(host, fixture, scheduler)?;
    anyhow::ensure!(!enabled(host), "portal near plane did not hide cached content");
    fixture.eye[2] = 1.5;
    fixture.dimension = 1;
    settle(host, fixture, scheduler)?;
    fixture.dimension = 0;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(enabled(host) && visible_state(host, glowstone) && !visible_state(host, netherrack), "revisiting did not refresh destination snapshot");
    fixture.dimension = 1;
    fixture.unknown = true;
    fixture.revision += 1;
    settle(host, fixture, scheduler)?;
    fixture.dimension = 0;
    fixture.unknown = false;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(!enabled(host), "unknown destination was retained as a complete snapshot");
    fixture.dimension = 1;
    settle(host, fixture, scheduler)?;
    fixture.dimension = 0;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(enabled(host), "complete revisit failed to recover");
    fixture.registry += 1;
    fixture.revision += 1;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(!enabled(host), "old registry runtime IDs remained visible");
    fixture.connection += 1;
    settle(host, fixture, scheduler)?;
    anyhow::ensure!(!enabled(host), "another connection inherited snapshots or portal links");
    println!("PASS: packed cached guest, zero local-terrain grants/source, directional links, runtime block states, stale snapshot while away, refresh on revisit, pause, near plane, unknown cells, registry and reconnect fences");
    println!("Native grids={}, triangles={}, peak callback fuel={}. GPU completion simulated.", fixture.native.grids, fixture.native.triangles, fixture.peak);
    Ok(())
}
