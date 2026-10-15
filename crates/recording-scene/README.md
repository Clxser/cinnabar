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
and mesh allocations once. Weak caches reuse admitted objects without keeping all
discarded scenes alive. Cancellation reaches bounded chunks of large asset reads;
cancelled or invalid data discards the draft. The parent capture manifest records
each track's schema, local directory and completion status. An incomplete track
stays distinguishable from a finished POV video when the take is reopened.

This seek operation is not an incremental playback loop. Full received-world actor coverage,
block entities, particles, an isolated replay viewport,
camera switching and preview/export composition still need implementation. Terrain
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
for a different camera location, environment interpolation, shader selection and
replay renderer publication still need implementation and runtime acceptance.

## Prepared actors

`actors.cinreplay` stores prepared GPU instances, previous/current bone matrices,
exact draw identities, geometry spans, skin residency and entity/equipment/glint
rasters. Immutable geometry and pixel objects use checksum-addressed storage;
pose samples are compressed inside the journal rather than creating a file per
frame. Queue bytes and lifetime-pinned geometry/appearance have separate budgets.
The producer neither hashes pixels nor copies geometry. Startup, frame-cadence
sampling, worker errors and manifest completion are connected in the app.

Restore validates native instance/bone limits, matrix finiteness, geometry and
raster references before returning a separate frame. These prepared draws retain
the original camera's culling and omit its hidden first-person body. Coverage is
explicitly `OriginalView`; this does not establish unrestricted replay cameras.
Unculled actor preparation, pose interpolation and native replay publication remain
incomplete. Appearance pinning is capped at 512 MiB; capture stops with an error if
that limit is exceeded. Aggregate take budgets and performance remain unverified.
