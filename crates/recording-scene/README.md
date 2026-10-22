# Native recording scenes

This Bevy-free owner freezes terrain, environment and actor draws actually prepared by the client. It does
not own an authoritative world, a network session, a camera or a GPU. The app
adapter lends immutable prepared geometry; recording does not run a second mesher.

## Capture

`TerrainCapture` admits one baseline and ordered change batches without filesystem
work or compression on the producer. Native change filters collect replaced
meshes and unloads. Material or tint replacement starts another frozen palette.
The worker stores cube geometry and lighting, model instances and draw partitions,
liquid geometry and lighting, and exact biome halo records. Per-layer UV metadata,
random model offsets, material tables, textures and complete mip chains survive
independently of the running install. Ordinary startup-carrier validation retains
its original limits.

The terrain journal lives in `terrain.cinreplay` inside its parent recording.
Schema `cinnabar.terrain/2` records the selected material/tint palette separately
from chunk bindings, including empty worlds and pack changes without new geometry.
Every checkpoint retains that selection. The experimental terrain/1 schema lacks
this state and is rejected for native playback; its parent POV video is unaffected.
Content-addressed objects deduplicate immutable assets. A checkpoint references
already stored objects, avoiding another terrain or texture copy. Palette identities
use bounded weak caches; queuing charges geometry and palette allocations for their
retained lifetimes. An exhausted queue, memory budget or disk budget stops capture
with an error. It never silently drops terrain changes or waits on the game thread.
Object and journal disk budgets are explicit. Overall video/audio/scene quota
coordination and measured startup/streaming costs remain incomplete.

## Restore

`TerrainRecording::restore` starts at the nearest complete checkpoint on a worker.
It validates object hashes, stream bounds, model references, texture layouts,
biome indices, liquid partitions and checked world origins before returning an
independent staging scene. Its memory ledger charges shared world, tint, palette
and mesh allocations once, including the active palette when no chunk references it.
Weak caches reuse admitted objects without keeping all
discarded scenes alive. Cancellation reaches bounded chunks of large asset reads;
cancelled or invalid data discards the draft. The parent capture manifest records
each track's schema, local directory and completion status. An incomplete track
stays distinguishable from a finished POV video when the take is reopened.

`TerrainPlayback` reuses the source index and object cache for forward playback.
Its successful patches contain only changed bindings and unloads. Later redundant
terrain checkpoints do not clear the current scene; seeks return a replacement
patch. Failed or cancelled drafts are invalidated before another frame can use them.
The renderer must retain its last published state until it admits the next patch.

Full received-world actor coverage,
block entities, particles, camera controls and complete preview/export composition
still need implementation. Terrain
capture alone is not a complete world replay. No build or runtime acceptance has
been performed under the current source-edits-only instruction.

## Environment track

`environment.cinreplay` retains the prepared atmosphere, lightmap, fullbright and
texture animation clock. The app samples these at the video's frame cadence.
Each sky asset epoch is pinned once and serialized on the existing journal worker;
pixel arrays are not cloned on the game thread or repeated in state records.
Frozen atmosphere assets include runtime pack replacements and fog/biome profiles.
Optional skybox faces preserve the owner's dimension and face order. Ordinary
startup validation keeps its existing provenance and texture-size checks.

`EnvironmentRecording::restore` validates state and checksum-admits the referenced
assets on a worker, with cancellation during large reads. State checkpoints and
asset objects remain separately bounded. Completion or failure is included in the
parent manifest. This preserves the recorded camera's prepared inputs. Relighting
for a different camera location, environment interpolation and shader selection
still need implementation. Native renderer publication has source wiring and
remains unverified.

## Prepared actors

`actors.cinreplay` stores prepared GPU instances, previous/current bone matrices,
exact draw identities, geometry spans, skin residency and entity/equipment/glint
rasters. Immutable geometry and pixel objects use checksum-addressed storage;
pose samples are compressed inside the journal rather than creating a file per
frame. Queue bytes and lifetime-pinned geometry/appearance have separate budgets.
The producer neither hashes pixels nor copies geometry. Startup, frame-cadence
sampling, worker errors and manifest completion are connected in the app.

Restore validates native instance/bone limits, matrix finiteness, geometry and
raster references before returning a separate frame. New takes publish received
actors and the local world body before camera culling; older `OriginalView` takes
retain their original coverage. Continuous poses interpolate within compatible
lifetimes. Rendered acceptance remains open.

Queued asset epochs share charges for identical immutable allocations. A skin or
wrapper change does not charge unchanged texture/vertex pages again. Dropping the
last lease releases each charge before its corresponding allocation. Snapshot and
total-pinned limits remain enforced; errors include retained, requested and limit
bytes. Aggregate take budgets and performance remain unverified.

## Coordinated playback

`ScenePlayback` admits terrain, environment, actors and the original-view camera
as one frame on a worker. It rejects mixed take metadata and bounds interrupted
playback to the common committed duration. Forward playback retains each cursor;
actor seeks decode only the final crossed pose. Geometry, rasters and sky objects
reuse checksum-admitted epochs. A failed track invalidates every cursor, and a
successful seek explicitly marks temporal histories for reset.

The native editor can select a captured POV clip and use its 3D replay through a
reversible edit. `render::replay` owns a separate headless scene on the shared GPU
device. It publishes recorded terrain, prepared actors and environment without
replacing live-world resources. Free/path shot values can supply a camera pose;
target tracking is still missing and disclosed. Seeks replace camera view history.
A four-target lease pool and GPU-only UI copies avoid frame readback and prevent
reuse before display/copy completion. Device errors remain visible to the primary
application even after the editor closes.

This provisional viewport supports one unframed replay layer. Native/raster layer
composition, replay transitions/crops, full camera controls, audio, titles, global
effects, environment/pose interpolation, shader-mode restoration, exact recorded
projection planes, HUD/hand and full received-world actor coverage remain
incomplete. No build, test, rendered parity or performance acceptance has been run.
