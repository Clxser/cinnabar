# Recording Studio

The studio keeps immutable recordings separate from editable project metadata.
This crate owns timeline time, project admission, undoable commands, camera rigs,
automatic edit proposals, local storage and worker-owned video encoding. It does
not own a live game session, GPU resources or the replay renderer.

## Projects and editing

`.cinstudio` files describe source media, video tracks, camera shots, effects,
audio, titles, markers, transitions and export settings. `Editor` validates and
commits an edit batch atomically. Undo history shares project snapshots and never
copies media. `CompiledTimeline` indexes intervals once per metadata revision and
reuses a caller-owned `FramePlan` during preview and export.

Linked game audio follows its visual clip through cuts, moves, trims, speed
changes and deletion. Its gain, mute and outer fades remain independent mix
controls. Unlinked music and voice tracks keep their own timeline anchors.

All timing uses integer microseconds and rational frame/playback rates. The caller
supplies elapsed time; seeking changes the transport epoch so a presentation
adapter can reset interpolation, temporal graphics and audio histories together.

Combat observations distinguish confirmed attacker attribution from correlation
with attempted attacks. Automatic effects are ordinary editable commands. A
proposal reports omitted highlights when the current edit budget cannot fit them;
it must not be presented as complete analysis of the recording.

## Recording and recovery

`Recording::start` launches a bounded storage worker. `push` transfers an immutable
payload without disk IO or compression on the producer. Callers enclose the
initial world snapshot in `CheckpointStart`, `CheckpointPart` and `CheckpointEnd`
records at time zero. Later checkpoints use the same multipart form; ordinary
scene, camera, gameplay, audio and asset records retain capture order between them.
Adapters should checkpoint periodically to bound seek work.

The scene adapter owns the payload schema and records its identifier in
`RecordingInfo::scene_schema`. The adapter must reject incompatible schemas before
interpreting payloads. Only scene data the client actually received can appear in
a recording; the container does not create unseen terrain or infer missing facts.

The `.cinreplay` directory contains metadata, compressed segments and an append-only
index. Each segment is finished and synced before its index entry is published.
Checksums, byte limits, sequence checks and checkpoint framing are verified while
reading. An incomplete index tail and unindexed `.part` files do not become replay
state. An interrupted recording can reopen its committed prefix.

`finish` closes capture admission immediately and returns an observable task. The
worker drains accepted records and publishes completion. Full queues stop capture
with a visible error, keeping the committed prefix; they do not block live play or
silently omit scene changes. Disk quotas also apply during compression, including
large multipart checkpoints. Worker progress distinguishes processed data from
durable commits.

`RecordingReader::visit_to` starts from the nearest complete checkpoint. A seek
builds a staging scene on a worker. The adapter publishes it only after successful
validation; cancellation or corruption discards the draft.

`RecordingCursor` retains a verified segment and one lookahead header for forward
playback. Each record crosses the visitor once; seeking explicitly resets the
cursor. Complete framing and checksum validation precede the first callback from
each segment. Small segments remain in bounded memory; large checkpoints use an
anonymous, delete-on-close scratch file. Cancellation reaches compressed reads,
decoded copies and visitor traversal. A failed operation requires a fresh seek.

POV captures periodically checkpoint their settings and actual camera pose using
the same interval as native scene tracks. `CameraRecording` restores the recorded
view without borrowing the live camera or inventing one for a missing sample.
Initial settings-only checkpoints from earlier captures remain readable. Camera
interpolation uses recorded timestamps; discontinuity and cut/teleport acceptance
still need qualification.

`object_store` owns bounded, SHA-256-addressed immutable assets. Object publication
uses exclusive staging; readers verify the whole digest and reject trailing bytes
before returning decoded state. Native terrain serialization lives in the separate,
Bevy-free `recording-scene` crate rather than coupling project editing to game state.

## Project storage

`ProjectStorage` owns bounded asynchronous save, open and library jobs. Saves write
and sync an exclusive sibling staging file before replacing a project. Relative
media paths preserve their meaning across Save As. A completion carries its job
and project revision so an older save cannot mark newer edits as saved. Library
enumeration reports invalid files and truncated listings without opening world
payloads.

The native studio route now presents a library, metadata editor and timeline.
Main-screen recording can arm the next world; pause and studio controls start,
stop and bookmark a take. Native-only capture skips video encoding and pixel
readbacks. Optional video capture uses a bounded readback pool, bounded live
frame admission, a passive mixed-audio tap and background storage/encoding.
Hardware encoding is probed on a startup worker, with software fallback in Auto.
Missing video slots repeat the preceding frame within a bounded gap; missing
audio blocks become silence at their recorded offsets. Both tracks share a final
video-frame boundary. Capture failures remain separate from the live game session.

The live adapter retains a camera journal identified by `POV_SCENE_SCHEMA`,
committed combat observations and independent native tracks. Video capture adds a
POV MP4. Terrain capture shares prepared
geometry, records changes and unloads, and freezes exact local runtime render assets
on a bounded worker. A separate environment track retains prepared sky/weather,
lighting and texture-animation inputs plus frozen sky assets. A prepared actor
track retains rig poses, skins and artwork with explicit captured actor coverage.
An independent received-actor target track records anchors, angles, names and spawn
identities, including the local subject's published eye position. Its changed-only
records and periodic checkpoints share the take clock; target tracking does not
expand the actor draw track's coverage.
Track completion and failure are saved independently of POV completion.
Opening a native-only package creates an editable 3D replay clip. A video package
makes ordinary video/audio clips and editable highlights. A
captured clip can switch to its independent scene through **Use 3D replay**. A
raster video alone remains a POV recording.

## Media preview

The native editor now requests raster frames from one bounded background worker.
It coalesces forward requests and cancels older seeks/documents, retaining no
unbounded scrub backlog. FFmpeg decodes local video and images through a one-frame
handoff; owned subprocesses stop on cancellation or editor closure. Adjacent video
requests retain their decoder, and metadata edits can retain unchanged media.
The frame/input thread does no process waiting, decoding, composition or pixel
hashing. Missing FFmpeg and failed media appear as preview errors.

The current raster compositor handles up to eight simultaneous sources, authored
crops, opacity, ordered tracks, black gaps and grouped dissolves in linear light.
An unchanged opaque single-source frame shares its decoded pixels directly. A
dedicated native UI slot preserves aspect ratio and ordinary menu/modal ordering;
it is outside server and mod texture ranges. The page remains allocated while a
seek is loading, avoiding repeated texture-plan changes during a drag.

Source frame accuracy, crop filtering, source identity, audio synchronization and
title composition still need integrated hardware acceptance.
The render edge now applies timeline effects through the shared GPU path below.
Missing 3D camera composition is disclosed for ordinary video. A selected captured
clip can switch reversibly to a native
scene source through **Use 3D replay**. The render edge owns its isolated GPU scene
and leased image publication; this domain owner retains no device or live world.
Direct spatial authoring requires a single unframed replay layer. Preview reports
the recorded actor and artwork coverage instead of inferring missing scene data.

## Replay cameras

The native inspector creates player, free, path, follow and orbit shots from the
displayed replay frame. Shot metadata uses normal undo/redo and project storage.
Free/path controls edit position, angles and field of view; follow controls edit
offset and velocity look-ahead; orbit controls edit radius, height, phase and turns.
Fine adjustment uses smaller steps. Keyed path adjustments insert or replace a
key at the exact playhead, preserving an existing key's easing. Whole-rail travel
requires an explicit switch to Key easing before exact XYZ edits or pose-key
insertion. Start/end trims retain the path's original motion clock and support
geometry, independent FOV phase and the orbit's phase.

Target browsing and retargeting are separate operations. Newly authored actor
shots bind a recorded spawn lifetime, preventing runtime-ID reuse from selecting
a different subject. Player mode requires a recorded native eye offset; received
remote eye views are available only for the captured native poses described in
the user guide. Missing authored subjects produce an explicit preview error while
retaining the last admitted image, with retargeting still available. Suggestions
require a continuous recorded subject interval; export verifies actual required
output samples and keeps one canonical spawn identity across the shot.
Older takes without the optional target track can still use free/path shots.

Both preview paths evaluate the same output-frame grid. Camera-only metadata edits
retain admitted source cursors and terrain; explicit seeks reset temporal history.
Whole-rail motion, numerical approximations, trim phase and timing-mode contracts
are documented in [camera/README.md](src/camera/README.md). Named source-bound
camera views are stored with ordinary undoable project metadata; recall remains
transient. Save As, open and recovery relocate bindings with source paths without
rebinding changed media. Camera smoothing, remote POV poses and rendered/input
acceptance across narrow layouts remain open.

The editor is experimental. Source implementations and owning regression tests do
not establish sustained capture performance, complete scene coverage or an
accepted playback/export workflow. The public plan keeps those acceptance gates
open until recordings, navigation, audio, exports and recovery are exercised on
the target hardware.

## Timeline effects

The native inspector creates each supported effect and edits manual and generated
effects through the same undoable commands. Select an effect on the timeline to
enable/disable it, adjust its amount at the playhead, add/remove exact-time keys,
cycle incoming easing, navigate keys (including clip-end keys), or move the effect
one output frame. Fine adjustment uses smaller amount steps. A bounded graph
samples the actual curve; dense timeline key markers share visible columns, while
key navigation retains access to every key. Gameplay provenance survives edits.
Generated effects with source anchors follow their retained visual clip's clock;
legacy provenance without anchors stays unchanged.

Overlapping cameras, effects, audio and titles now receive separate selectable
presentation lanes. Packing and source-name lookup are cached per project
revision; playback does not repack them. Offscreen rows/items are culled before
text layout. These lanes do not change compositing or camera precedence.

The render edge shares original GPU shaders across preview and export for linear
exposure, saturation, vignette, bloom, flash, letterbox and fades, including
dip-to-black transitions. Bloom uses bounded low-resolution extraction and a
separable blur; neutral frames retain their direct path. GPU targets, blur scratch
and raster upload surfaces are reused. Output backpressure is checked before
uploading pixels, and native preview needs no effect readback. The Rust uniform
schema generates its WGSL declaration to keep binding layouts synchronized.
Paused replay edits reuse an unchanged base image when source time and camera
pose match, while seeks still reset temporal state. Failed scene application or
drawing invalidates source admission before a retry.

Replay zoom/shake moves the actual recorded world camera. Ordinary video instead
uses a deterministic affine crop/translation calibrated by a shared virtual FOV;
it cannot reveal unrecorded pixels or reproduce 3D parallax. This image motion and
the SDR bloom/look remain provisional. Composition, keyframe editing, native
crossfades/crops/layering, narrow layouts, rendered appearance, preview/export
equality and cost still need integrated hardware acceptance.

## Video export

**Export** freezes the current metadata revision and writes an MP4 into the Studio
user-data directory's `exports` folder. Further edits do not change that job. The
native progress banner remains available in the library and editor, with Cancel
available until final file publication begins. Export does not replace existing
files; directory exports choose a unique `Export N.mp4` name.

The export worker evaluates every planned frame on the same rational grid as
preview. It shares the CPU/NVIDIA/Auto codec policy, reports the actual selected
encoder, checks frame ordering and dimensions, and waits for successful encoder
flush. Raster cuts, crops, opacity and dissolves use the preview compositor. A
single native replay layer uses the isolated renderer and authored camera rigs at
the requested output size, subject to the frame/device memory limits. Reusable
GPU readback and CPU storage keep one native frame in flight. Device polling stays
with the normal render owner; Cinnabar must keep rendering during export.

Audio export decodes one source at a time into bounded blocks, sums it into
disk-backed stereo float storage, and muxes the finished mix with the video. Clip
trims, rational speed changes, gain, mute, fades and silent tails use the saved
timeline. Tempo stages use FFmpeg's [atempo](https://ffmpeg.org/ffmpeg-filters.html#atempo)
filter within its blending range. Audio boundaries quantize to the 48 kHz sample
grid; source accuracy, tempo latency and synchronization still need acceptance.
Scratch audio is capped at 16 GiB; video output at 64 GiB. Staging files and scratch
directories are local, ignored artifacts and normally removed on cancellation or
failure. A supervised owned child keeps cancellation effective during blocked
encoder/decoder pipe IO. No UI thread joins these workers.

This path remains provisional. Titles, native/raster layering, crops, crossfades
and effects share the preview compositor. Camera shots require native replay
coverage. The complete edit is checked before encoding; requested stream metadata
and a full payload decode are verified before final publication.
Unavailable camera subjects can still fail later payload admission. Native shading
and recorded actor-coverage limitations are displayed in the status. Export settings
controls, destination selection, completed-file actions, interrupted-job recovery,
full visual composition and synchronized audio preview need integrated acceptance. A
finished-file path in source code does not establish an accepted export workflow.
