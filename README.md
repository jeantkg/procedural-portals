# Procedural Portals 0.4 — visited terrain

A temporary, client-only portal preview for existing Cinnabar builds with
`cinnabar:world-draw@0.2.0`. It uses blocks you have actually visited, rather than
generated scenery. No client patch, server plugin, custom launcher, or manually
started server is required. It does not import the proposed `local-terrain` API.

Download the WASM and settings companion or the ZIP from
[GitHub Releases](https://github.com/jeantkg/procedural-portals/releases).
Version 0.4 is a preview release; in-game visual acceptance is still pending.

## Use it

1. Import `procedural-portals.wasm` through **Mods**, enable it, and allow Terrain,
   Player state, Settings, World drawing and Procedural materials. Allow HUD to see
   the short cache/preparation messages. Disable older copies of this mod.
2. Join your normal world and go through a lit Nether portal normally.
3. Step out of the opening and wait near it for a few seconds while nearby
   chunks load. Keep only one portal close to you while learning the connection.
4. Return through that portal normally. After preparation, the original portal
   shows the destination blocks captured during your visit. Move sideways to see
   the perspective. A further crossing learns the other direction as well.

Before a direction and a complete destination snapshot have been observed, its
preview stays hidden and the normal portal remains. No fictional scenery is
silently substituted. Minecraft retains control of portal travel and its overlay.

Snapshots update while you are near the portal in that dimension. They **do not
update live while you are in the other dimension**. Revisit to refresh changes.
Closing the connection, reloading the mod, dying, or changing the block registry
clears the cache; repeat the normal trip afterward. Ordinary multiplayer can use
this same visited-terrain approach, subject to the server sending nearby chunks.

## Settings

The optional `procedural-portals.settings.json` sits beside the WASM. Start with
the supplied companion and reload the mod after edits. Missing keys inherit
the checked-in `default.settings.json`.

| Setting | Meaning |
| --- | --- |
| `enabled` | Enable the visual mod. |
| `cached_terrain` | Use visited blocks; `false` explicitly selects the original fictional scenes. |
| `search_distance` | Portal search radius in blocks, from 8 to 112. |
| `max_portals` | One or two simultaneous previews. |
| `ambient_light` | Approximate preview lighting, from 0 to 15. |
| `show_status` | Short HUD preparation and cache messages. |
| `quality`, `seed` | Low/medium/high size and seed for fictional scenes only. |

The 0.3 keys `live_terrain` and `load_destination_chunks` are accepted for migration.
`live_terrain` maps to `cached_terrain` unless the latter is supplied. This build
never requests loading or ticking destination chunks.

## Windows build

Clone this repository, then create a separate sibling Cinnabar checkout for its
public Rust SDK. `sdk.json` pins the source revision used by this project. These
commands assume that `../cinnabar` does not already exist; preserve any existing
checkout and use a fresh parent directory if necessary.

```powershell
git clone https://github.com/jeantkg/procedural-portals.git
cd procedural-portals
$sdk = Get-Content sdk.json -Raw | ConvertFrom-Json
git clone $sdk.repository ../cinnabar
git -C ../cinnabar checkout --detach $sdk.revision
```

Install Rust and the toolchain/target in `rust-toolchain.toml`. Then run:

```powershell
.\Build.ps1
# If MSVC link.exe is unavailable and the matching GNU toolchain is installed:
.\Build.ps1 -Toolchain 1.99.0-x86_64-pc-windows-gnu
```

The script tests this mod, builds its WASM and validates/packages the component
into `dist/procedural-portals.wasm` and a versioned ZIP. It does not build Cinnabar.
The packer rejects accidental imports of the proposed `local-terrain` interface.
No engine patch is included in the 0.4 ZIP.

If PowerShell blocks local scripts, invoke them with a process-scoped policy:
`powershell -NoProfile -ExecutionPolicy Bypass -File .\Build.ps1`.

To exercise the packed guest through the native mod host and actual terrain mesher:

```powershell
.\Verify-Host.ps1 -Component .\dist\procedural-portals.wasm -Toolchain 1.99.0-x86_64-pc-windows-gnu
```

That optional developer check builds only a `mod-host` example, uses the matching
local world asset carrier, and simulates GPU completion. It does not render a game
frame or compile the client.

## Limits

- This is a snapshot preview, not a second live dimension camera. Each snapshot
  covers a 16-block cube; the far half is drawn through the portal's native clip.
- Links are last-observed, directional associations, not the server's authoritative
  portal resolver. Nearby competing portals or server-directed travel can make an
  association wrong. Cross again to relearn it.
- At most eight portal snapshots are retained in memory. Oldest-created entries
  are evicted; there is no disk cache or sharing between worlds/connections.
- Unknown chunks, missing block metadata, oversized palette metadata and incomplete
  openings hide the preview. Large portals must fit the sampling cube and collar.
- Samples are gathered over several frames, so a snapshot is not an atomic world
  capture. Primary block layers only; no entities, block entities, waterlogged
  secondary layers, biome tints or solved destination lighting are captured.
- Sky and lighting are approximations. There is no recursive portal rendering,
  instant teleport, or guarantee of a seamless visual crossing.
- Native-host tests establish activation, resource flow and frame-fuel behavior.
  Real client rendering, GPU staging, resource packs and frame timings still need
  in-game acceptance. No modified client executable is supplied.

See [cache implementation notes](docs/cached-terrain.md).
All fictional scene code is original; no Immersive Portals assets are included.
