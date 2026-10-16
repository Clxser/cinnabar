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
interpolation between samples and cut/teleport handling still need implementation.

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
stop and bookmark a take. Capture uses one GPU readback at a time, bounded live
frame admission, a passive mixed-audio tap and background storage/encoding.
Hardware encoding is probed on a startup worker, with software fallback in Auto.
Missing video slots repeat the preceding frame within a bounded gap; missing
audio blocks become silence at their recorded offsets. Both tracks share a final
video-frame boundary. Capture failures remain separate from the live game session.

The current live adapter writes a POV video package, explicitly identified by
`POV_SCENE_SCHEMA`. It includes camera metadata and committed combat observations,
with a separate native terrain journal alongside it. Terrain capture shares prepared
geometry, records changes and unloads, and freezes exact local runtime render assets
on a bounded worker. A separate environment track retains prepared sky/weather,
lighting and texture-animation inputs plus frozen sky assets. A prepared actor
track retains rig poses, skins and artwork with explicit original-view coverage.
An independent received-actor target track records anchors, angles, names and spawn
identities, including the local subject's published eye position. Its changed-only
records and periodic checkpoints share the take clock; target tracking does not
expand the actor draw track's coverage.
Track completion and failure are saved independently of POV completion.
Opening a package makes ordinary video/audio clips and editable highlights. A
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

This is an unverified source implementation. Crop filtering, source frame accuracy,
content-identity validation, audio, titles and global effects still
need completion and acceptance. Missing camera/effect composition is disclosed in
the raster preview. A selected captured clip can switch reversibly to a native
scene source through **Use 3D replay**. The render edge owns its isolated GPU scene
and leased image publication; this domain owner retains no device or live world.
That provisional path currently supports a single unframed replay layer and
discloses its original-view actor coverage and missing composition features.

## Replay cameras

The native inspector creates player, free, path, follow and orbit shots from the
displayed replay frame. Shot metadata uses normal undo/redo and project storage.
Free/path controls edit position, angles and field of view; follow controls edit
offset and velocity look-ahead; orbit controls edit radius, height, phase and turns.
Fine adjustment uses smaller steps. Path adjustments insert or replace a key at
the exact playhead, preserving an existing key's easing. Start/end trims retain
boundary poses and interior keys or the orbit's phase; trimming a path can change
its interior interpolation.

Target browsing and retargeting are separate operations. Newly authored actor
shots bind a recorded spawn lifetime, preventing runtime-ID reuse from selecting
a different subject. Player mode requires a recorded native eye offset; received
remote actors currently provide centre/root tracking only. Missing subjects show
an explicit notice and the original view, with retargeting still available.
Older takes without the optional target track can still use free/path shots.

Both preview paths evaluate the same output-frame grid. Camera-only metadata edits
retain admitted source cursors and terrain; explicit seeks reset temporal history.
Target motion interpolation, camera smoothing, unculled actor draws (including the
local first-person body), visible key markers/easing controls and inspector access
in narrow layouts remain incomplete. These controls have not been run or visually
accepted.

The full scene schema, complete native replay rendering/preview composition, media
import/recovery, complete camera/effect inspectors, recording options, export and linked
automatic-effect retiming are still being implemented. The feature has not been
built or exercised. Media preview and a limited native replay viewport are connected
in source. These pieces do not complete the feature.
