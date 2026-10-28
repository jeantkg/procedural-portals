# Visited-terrain cache

Version 0.4 uses only the existing `terrain.read-region`, `terrain.find-blocks`,
player observations, budget, settings, HUD and world-draw APIs. Destination terrain
is never queried while the player is in another dimension.

The detector verifies a complete loaded portal opening and its collar. One selected
opening is sampled per frame in a 16 x 1 x 16 slab. Sixteen complete slabs publish an
immutable 16-cube snapshot. Search plus up to two opening checks plus the slab fit
the host's four-query limit. After a consumed slab, capture yields one frame to
preview preparation. Pending replies renew subscriptions; snapshots with
unknown primary cells or missing identifiers are discarded. Geometry is not requested.
Each snapshot allows at most 128 KiB of retained identifier/portal-state metadata.

Runtime block IDs preserve the actual observed block variants, including orientation.
They are scoped to the world connection and exact registry generation. Registry
changes discard both cached snapshots and preview geometry. Palette canonical
states are retained only for validating the portal axis; other rendering uses the
observed IDs. Cache-owned revisions change only when the assembled snapshot changes.

Capture continues while the portal is selected, regardless of camera direction.
The cache stores eight portals and evicts the oldest-created entry. It is memory
only, survives dimension-loading gaps in the same connection, and is cleared on
observed death or a different world/connection. Mod reload reconstructs an empty cache.

Normal observed crossings associate one unique nearby verified source with one
verified arrival. They do not infer the reverse link. Foot position, rather than
camera position, selects a crossing; camera position selects the visible side.
The existing proper rotation maps the destination into the source portal; the
destination portal blocks and near half-space are omitted. Original world blocks
in front of the portal occlude its native mesh clip.

Snapshot-to-grid packing is spread over frames. An immutable snapshot remains shared
while packing, so a partial capture cannot change a grid being prepared. GPU resource
replacement is staged. Ordinary pause retains rendering; invalid current portals,
loading gaps and changed registry generations release world-owned drawing resources.

The 0.3 server bridge remains a separate upstream proposal, not a dependency or
installation requirement of 0.4. Historical BDS probe tools are not used by this mode.
