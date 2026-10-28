//! Original block scenes. Their cells are presentation data and are never written to a world.
use crate::config::Config;
use mod_api::drawing::{self, api::*};

pub const FLOOR: u32 = 4;
pub const SKY_SHADER: &str = include_str!("sky.wgsl");

pub struct Inputs {
    pub nether: GeometrySpec,
    pub overworld: GeometrySpec,
    pub plane: GeometrySpec,
    pub sky: GeometrySpec,
    pub size: [u32; 3],
}

fn named(name: &str, states: &str) -> BlockState {
    BlockState::Named(NamedBlock {
        name: name.into(),
        states: states.into(),
    })
}

fn hash(x: u32, z: u32, seed: u32) -> u32 {
    let mut v = x.wrapping_mul(0x9e3779b9) ^ z.wrapping_mul(0x85ebca6b) ^ seed;
    v = (v ^ (v >> 16)).wrapping_mul(0x7feb352d);
    v ^ (v >> 15)
}

impl Inputs {
    /// Prepares packed words in the larger PlayerMod init allowance; frame callbacks only upload.
    pub fn new(config: &Config) -> Self {
        let size = config.quality.size();
        let empty = || {
            GeometrySpec::Blocks(BlockGrid {
                size: (1, 1, 1),
                palette: vec![named("minecraft:air", "{}")],
                cells: vec![
                    drawing::pack_block_cell(BlockCell {
                        palette: 0,
                        sky_light: None,
                        block_light: None,
                    })
                    .unwrap(),
                ],
            })
        };
        Self {
            nether: if config.cached_terrain {
                empty()
            } else {
                GeometrySpec::Blocks(grid(size, config, false))
            },
            overworld: if config.cached_terrain {
                empty()
            } else {
                GeometrySpec::Blocks(grid(size, config, true))
            },
            plane: GeometrySpec::Triangles(Mesh {
                vertices: [
                    (-0.5, -0.5, 0.),
                    (0.5, -0.5, 0.),
                    (0.5, 0.5, 0.),
                    (-0.5, 0.5, 0.),
                ]
                .into_iter()
                .zip([(0., 0.), (1., 0.), (1., 1.), (0., 1.)])
                .map(|(position, uv)| Vertex {
                    position,
                    uv,
                    color: (1., 1., 1., 1.),
                    data: (0., 0., 0., 0.),
                })
                .collect(),
                indices: vec![0, 1, 2, 0, 2, 3, 2, 1, 0, 3, 2, 0],
            }),
            sky: sky_box(),
            size,
        }
    }
}

/// Five inward-visible faces leave the portal-facing side open and close every other ray.
fn sky_box() -> GeometrySpec {
    let faces = [
        [
            (-0.5, -0.5, -0.5),
            (0.5, -0.5, -0.5),
            (0.5, 0.5, -0.5),
            (-0.5, 0.5, -0.5),
        ],
        [
            (-0.5, -0.5, 0.5),
            (-0.5, -0.5, -0.5),
            (-0.5, 0.5, -0.5),
            (-0.5, 0.5, 0.5),
        ],
        [
            (0.5, -0.5, -0.5),
            (0.5, -0.5, 0.5),
            (0.5, 0.5, 0.5),
            (0.5, 0.5, -0.5),
        ],
        [
            (-0.5, 0.5, -0.5),
            (0.5, 0.5, -0.5),
            (0.5, 0.5, 0.5),
            (-0.5, 0.5, 0.5),
        ],
        [
            (-0.5, -0.5, 0.5),
            (0.5, -0.5, 0.5),
            (0.5, -0.5, -0.5),
            (-0.5, -0.5, -0.5),
        ],
    ];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for face in faces {
        let start = vertices.len() as u32;
        for position in face {
            vertices.push(Vertex {
                position,
                uv: (0., position.1 + 0.5),
                color: (1., 1., 1., 1.),
                data: (0., 0., 0., 0.),
            });
        }
        indices.extend([0, 1, 2, 0, 2, 3, 2, 1, 0, 3, 2, 0].map(|i| start + i));
    }
    GeometrySpec::Triangles(Mesh { vertices, indices })
}

/// A closed Nether cavern, or rolling Overworld banks with water and naturally textured cliffs.
fn grid(size: [u32; 3], config: &Config, overworld: bool) -> BlockGrid {
    let [width, height, depth] = size;
    let palette = if overworld {
        vec![
            named("minecraft:air", "{}"),
            named("minecraft:stone", "{}"),
            named("minecraft:dirt", "{}"),
            named("minecraft:grass_block", "{}"),
            named(
                "minecraft:water",
                r#"{"liquid_depth":{"type":"int","value":0}}"#,
            ),
        ]
    } else {
        vec![
            named("minecraft:air", "{}"),
            named("minecraft:netherrack", "{}"),
            named("minecraft:soul_sand", "{}"),
            named(
                "minecraft:lava",
                r#"{"liquid_depth":{"type":"int","value":0}}"#,
            ),
            named("minecraft:glowstone", "{}"),
            named("minecraft:obsidian", "{}"),
        ]
    };
    // Encode only the small palette/light combinations, rather than allocating one record per cell.
    let words: Vec<_> = (0..palette.len())
        .map(|palette| {
            drawing::pack_block_cell(BlockCell {
                palette: palette as u32,
                sky_light: Some(if overworld { 15 } else { 0 }),
                block_light: Some(if !overworld && (palette == 3 || palette == 4) {
                    15
                } else {
                    config.ambient_light
                }),
            })
            .unwrap()
        })
        .collect();
    let mut cells = Vec::with_capacity((width * height * depth) as usize);
    for x in 0..width {
        for z in 0..depth {
            let noise = hash(x / 4, z / 4, config.seed);
            let entry = z >= depth - 5;
            let ground = if entry { FLOOR } else { FLOOR + (noise % 5) };
            let river_center = width as i32 / 2 + ((z / 6 + config.seed % 5) % 7) as i32 - 3;
            let river = !entry && (x as i32 - river_center).abs() < if overworld { 5 } else { 4 };
            let wall = x < 2 || x >= width - 2 || z < 2;
            let pillar = !entry && (x % 15 == 5 || x % 15 == 6) && (z % 17 == 4 || z % 17 == 5);
            let roof = height - 4 - noise % 3;
            for y in 0..height {
                let value = if overworld {
                    if y < FLOOR - 1 {
                        1
                    } else if river && y < FLOOR + 1 {
                        4
                    } else if !river && y < ground {
                        if y + 1 == ground { 3 } else { 2 }
                    } else if wall && z < depth - 8 && y < ground + (noise % 9) {
                        1
                    } else {
                        0
                    }
                } else if y < FLOOR - 1 {
                    1
                } else if river && y < FLOOR + 1 {
                    3
                } else if wall || y >= roof {
                    if (x / 3 + z / 3) % 13 == 2 && y >= roof {
                        4
                    } else {
                        1
                    }
                } else if pillar && y < roof {
                    if y < FLOOR + 2 { 5 } else { 1 }
                } else if !river && y < ground {
                    if noise % 7 == 1 { 2 } else { 1 }
                } else {
                    0
                };
                cells.push(words[value]);
            }
        }
    }
    BlockGrid {
        size: (width, height, depth),
        palette,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scenes_have_packed_native_blocks_light_and_open_entry_space() {
        let config = Config {
            cached_terrain: false,
            ..Config::default()
        };
        let inputs = Inputs::new(&config);
        for (spec, fluid) in [
            (&inputs.nether, "minecraft:lava"),
            (&inputs.overworld, "minecraft:water"),
        ] {
            let GeometrySpec::Blocks(grid) = spec else {
                panic!("native blocks")
            };
            assert_eq!(
                grid.cells.len(),
                (grid.size.0 * grid.size.1 * grid.size.2) as usize
            );
            assert!(
                grid.palette
                    .iter()
                    .any(|v| matches!(v, BlockState::Named(n) if n.name == fluid))
            );
            for &word in &grid.cells {
                let cell = drawing::unpack_block_cell(word).unwrap();
                assert!((cell.palette as usize) < grid.palette.len());
            }
            let index = (((grid.size.0 / 2) * grid.size.2 + grid.size.2 - 1) * grid.size.1
                + FLOOR
                + 2) as usize;
            assert_eq!(
                drawing::unpack_block_cell(grid.cells[index])
                    .unwrap()
                    .palette,
                0
            );
        }
    }

    #[test]
    fn seed_is_repeatable_and_changes_the_generated_scene() {
        let mut config = Config::default();
        let a = grid(config.quality.size(), &config, false);
        assert_eq!(a.cells, grid(config.quality.size(), &config, false).cells);
        config.seed += 1;
        assert_ne!(a.cells, grid(config.quality.size(), &config, false).cells);
        mod_render::material::compose(SKY_SHADER).unwrap();
    }
}
