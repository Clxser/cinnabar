# Recording Studio

Recording Studio is experimental. Capture, playback and export need hardware and
long-session validation; requested FPS is not a guarantee of sustained capture.

## Record a take

Open Recording Studio and its settings before recording. Capture defaults to
**3D replay**, which records editable scene data and game audio without a live
video encoder or GPU video readbacks. **3D replay + video** also saves the original
camera as an MP4 and requires FFmpeg during recording. Choose Recording FPS
(24, 30, 60, 120 or 240) and Recording size. New preferences use a 1080p limit;
Native keeps the window size. Size limits preserve aspect, never upscale, and use
GPU scaling before video readback. Keep the window size fixed during a take.

Hand geometry and HUD/chat capture are separate opt-in native replay channels.
With Record HUD and chat off, new POV video also excludes screen overlays while
leaving the live HUD visible. World labels remain part of the recorded world.
Stop recording and wait for finalization before opening its completed take. Missed capture slots repeat a
frame and are reported; use a smaller output or lower cadence if misses accumulate.
If a required native scene track fails, 3D replay recording stops and verifies its
common saved prefix in the background. In 3D replay + video, the original-camera
video can continue with an explicit incomplete-replay notice. Wait for saving to
finish before recovering the interrupted native take from the library.
Recording at 120/240 FPS gives slow-motion edits more source motion samples when
the scene preparation and storage workers sustain that cadence. The video option
also needs sufficient GPU readback and encoder capacity.

During capture, the status row shows the elapsed recording time. Detailed sample
and actor CPU times, GPU readbacks, encoder queues and stalls remain available
through developer-control diagnostics. These are capture health signals, not
end-to-end frame-time measurements. Native and scaled recordings
reuse bounded GPU readback buffers, and software live encoding limits its worker
count to leave capacity for the game. Automatic encoding probes NVIDIA support
before falling back to CPU. Sustained capture and the game FPS cost still need
measurement on the target machine.

## Workspace and preview

Settings separates Recording, Preview, Export, Setup and Controls into full-height
pages. Setup checks FFmpeg and FFprobe and actually encodes a private frame to
check the selected format's CPU and NVIDIA encoders. Its CPU fallback action
changes the project's export setting. Recording uses its own admitted preferences.
Replay and export coverage details appear in Setup. Pausing and seeking keep the
last received picture visible until its replacement arrives; opening a different
project clears it. Decode, render and audio failures, and actionable export
warnings, still appear beside their controls.

Preview quality affects the interactive viewer only:

| Quality | Maximum picture | Playback cadence |
| --- | --- | --- |
| Fast | 480 x 270 | Up to 30 FPS |
| Balanced | 720 x 405 | Up to 60 FPS |
| Full | 960 x 540 | Up to 60 FPS |

Preview never exceeds project cadence or upscales smaller output. Paused stepping
and seeking retain the exact project frame grid. A bounded recent decoded-frame
cache and reuse across adjacent cuts reduce repeated preparation. Export retains
the saved output resolution and rational frame rate. Opening Settings or Recovery
pauses playback and releases hidden preview work.

## Edit

Edit, Review and Audio layouts change panel visibility together. The viewer starts
large, while selecting an item exposes its properties. The Assist tab contains
separate Rhythm and Cameras sections for tempo grids and camera alternatives.
Reusable cinematic looks are in Effects. Rhythm estimates are approximate: listen, adjust tempo/offset,
then add the grid as one undoable edit. Camera alternatives become ordinary shots
only when added.

The camera viewport offers world-axis handles, grid snapping, composition guides,
frame-player and named saved views. Cameras separates Shots from Views so saved
framing does not sit below shot creation controls. Saved views retain exact
position, yaw, pitch, roll and lens with the project. Save, rename, delete and
explicit rebind are undoable; recalling a view changes only inspection. Relinked
recordings require explicit rebind when their captured identity no longer matches.

Key easing preserves authored pose timestamps. Distance timing keeps even travel
within each key interval; Whole rail travels over the complete geometric path
with separate travel easing. Orientation and FOV retain their own clocks. Exact
XYZ changes and inserting a pose key require switching Whole rail back to Key
easing. Curved path trims retain their original motion phase.

- Scroll over timeline tracks to move vertically. Shift+wheel, horizontal wheel,
  or wheel over the ruler pans through time. Ctrl+wheel zooms around the pointer.
- Use Fit for the whole project, Playhead to reveal the current frame, or drag the
  bottom overview to move the visible window. Follow tracks the playhead while playing.
- Drag a clip to move it; drag an actual visible edge to trim it. A drag commits
  once on release and can be undone. Ripple is an explicit timeline setting.
- Choose Media, Cameras, Effects, Text or Audio above the viewer. Properties shows
  only the selected item; visual clips have Clip, Frame, Speed and Cut groups.
- Compact/Comfortable changes Studio density independently of the game HUD.
  Timeline S/M/L reserves more track space; Focus view hides side panels and the
  timeline for a larger picture. Smaller
  windows keep one side panel beside the picture, with a persistent timeline
  outside Focus view and separate panel scrolling. Timeline height is also bounded
  by actual track content so empty space does not displace the picture.
- Fit view preserves the whole native picture, excluding composition letterboxes.
  Fill view covers the viewer without stretching and can crop its edges. These
  viewer preferences do not change export framing. Frame properties separately
  save clip rotation in degrees and a Fit/Fill layer mode for preview and export.
- The clip inspector accepts exact Position, Source In and Source Out seconds.
  Source fields preserve the explicit Position; Apply commits, Cancel discards.
- Add speed keys at the playhead, choose incoming easing, or browse keys with
  Prev/Next. Video, linked game audio and generated event edits share this clock.
  Preview and export use pitch-preserving tempo processing, including ramps;
  extreme changes can still introduce audible time-stretch artifacts.
- Click or right-click an unframed native replay viewport to focus camera navigation.
  Drag with left mouse to orbit the selected subject, or a point ahead of the
  camera when none is selected. Use WASD and Q/E to fly, hold right mouse to look,
  and hold Shift for faster movement. Escape releases focus. Apply saves the
  inspected pose to a Free/Path shot; Reset (or R) returns to the saved camera.
- Middle-drag pans in the frozen camera plane. Wheel dollies; Ctrl/Cmd+wheel
  changes the lens. Right-mouse+wheel changes flight speed. Views provides speed,
  optional smooth translation and Level horizon controls. These inspection
  preferences do not change the project until a view is applied.
- Navigate keeps the picture clear. Players enables small player-selection
  crosshairs, with labels only on hover; entity types come from recorded protocol
  metadata, not their display names. Older takes cannot classify remote subjects;
  Cameras offers an explicit All entities fallback for those takes.
- Path enables rails, keys and Bézier
  handles; drag a key or handle in the view plane and release to commit one undoable
  change. Auto handles restores the original smooth path. Direct spatial editing
  requires one native replay layer with default framing.
- Rotate exposes labelled yaw, pitch and roll rings for a selected Free shot or
  an exact Path orientation key. Drag a ring, then release for one undoable edit;
  Escape cancels. Angle labels open the same precise numeric controls as Properties.
  Rotation preserves the positional rail, lens and easing, including Whole rail
  timing. Small pictures retain numeric controls when the rings do not fit.
- F1 opens Controls, generated from the same command registry as keyboard input.
  Ctrl/Cmd+S saves and Ctrl/Cmd+E exports. Timeline-focused arrows step one frame,
  Shift+arrows step ten, S splits and F fits. Timeline shortcuts yield to focused
  camera navigation so backward flight does not split clips.
- Ctrl-click toggles timeline items; Shift-click selects a range. Ctrl+A selects
  unlocked items and Ctrl+D duplicates selected items with their linked edits.
  Delete and horizontal dragging operate on the selection. Each track has mute
  and editor lock controls. Mute is saved and affects both preview and export;
  lock prevents edits but is an editor preference.
- Snap aligns drags to nearby clips, markers, keys and the playhead; hold Alt to
  bypass it. Camera/effect/speed keys can be dragged in their timeline lanes.
  Effect and speed curves expose value changes vertically. Every drag commits once.
- Looks expand into ordinary editable effect curves. Individual color, grain,
  chromatic, soft-focus and sharpening effects can also be authored directly.
- Import audio/video/images or relink a selected source from Media. Output size,
  FPS, codec, encoder, audio and destination are available in settings.
  Replay HUD and chat defaults to Hidden and affects preview and export without
  removing world labels or project titles. Chat already baked into an existing
  video cannot be erased; use its 3D replay track or record a new clean take.
- Play auditions recorded game audio and imported tracks through the selected
  output device. Select an audio item for gain, mute and output-time fades.
  Seeking cancels old audio; playback waits for displayed video rather than
  allowing sound to run seconds ahead. The editor admits up to 32 simultaneous
  audio streams. Device buffering can add latency; this is not a DAC timestamp.

## Replay reference controls

The [Flarial release trailer](https://www.youtube.com/watch?v=tYpugXwzHq0)
provides a feature reference. Studio retains its native editor, effects, audio,
timeline and export workflows.

- Cameras includes stationary **Track entity** alongside Follow, Orbit, Free and
  Path. Creating Follow or Orbit retains the inspected framing. **Become entity**
  uses a recorded eye anchor and hides that actor's own body. New takes support
  standing, upright, unit-scale, living, unmounted players. Unsupported poses and
  older takes without eye data remain unavailable. Remote pose-transition eye
  easing and arbitrary mob POV are incomplete.
- Independent FOV keys animate a shot's lens with every rig. Keys support timeline
  dragging, easing, selection, deletion and undo. Trims retain their curve phase.
- Choose **Curves** to edit X/Y/Z, yaw/pitch/roll, FOV, effect strength or clip
  speed. Drag keys and the selected segment's weighted Bezier handles, or type a
  key's timeline seconds and value. Arrow keys nudge the selected key; Shift uses
  ten steps. Delete removes the key. Wheel zooms values, Shift-wheel pans time,
  and Ctrl-wheel zooms time around the pointer. Fit restores the curve bounds.
  Subject-driven camera graphs show offsets from the recorded rig; free/path
  graphs show composed world values. Handles shape scalar offsets over the
  original rail, preserving its geometry and travel clock. Value handles stay
  within their segment's endpoint envelope. A drag commits one undo entry;
  Escape cancels it. Unchanged plots and distance geometry are retained.
- A camera shot's **Entry** controls select Cut or an explicit timed blend.
  Compatible adjacent rigs blend position, lens and shortest quaternion rotation
  before global camera effects. Smooth easing carries outgoing authored motion
  through the entry. Blends require a continuous, unambiguous native replay clip;
  gaps, source cuts, missing subjects and reused spawn identities retain a cut.
  Hold keys, authored corners and pitch clamps can still be discontinuous. Camera
  blends do not provide terrain collision avoidance.
- Viewer controls provide frame steps, five-second jumps, start/end, typed seconds
  or `hh:mm:ss.ffffff`, whole-project looping and temporary monitoring speed from
  0.1x to 8x. Monitoring does not retime clips or change export. Audio uses
  pitch-preserving stretching. Slow playback retains the source frame grid; fast
  playback respects the interactive rendering budget. Loops re-prime preview and
  audio at the start; gapless looping is not guaranteed.
- Export settings accept a full project or saved In/Out on the project clock.
  Out is exclusive. Only output-frame timestamps inside the bounds render; audio
  uses its matching PCM interval. Type values or take them from the playhead.
  These undoable bounds remain separate from Source In/Out and monitoring loops.
  After shortening a project, reset or edit stale export bounds.
- Library search filters names and filenames before the 256 visible-item limit.
  Search runs on Apply/Enter. Ordering offers most recently modified, oldest
  modified or name. Enumeration remains bounded to 4096 directory entries;
  incomplete scans and result sets display a warning.
- The trailer does not specify its unopened Module controls. Studio uses existing
  keyframed Effects for that workflow. Undocumented module parity remains open.

Owning domain and Studio shell/UI regressions pass. Isolated UI snapshots exercise
camera and transport controls. A local Windows Vulkan pass exercises native
camera transitions, linked audio, seeking and range export. Broader synchronization,
long-session recovery and sustained recording performance still need qualification.

## Autosave and project recovery

Dirty projects checkpoint a separate recovery copy every 30 seconds. The store
keeps up to three versions of each of eight projects. It does not evict another
project to make space; the error offers Recovery and Retry. Studio discovers
recoverable projects when opened. Open Project recovery from the library or Setup,
then restore a copy or explicitly discard its versions.

Restoring opens a dirty, untitled copy; saving it creates a new project rather
than overwriting the original saved file. Project recovery preserves edit data
and media references, not duplicate media. Missing or moved source files still
need relinking. Explicitly discarded revisions remain retired even if cleanup is
interrupted. A retained editor after a canceled replacement offers Retry autosave
into a fresh recovery slot.

## Export safety

Export validates used source paths and recorded hashes where available before
rendering. Older unhashed media remains readable without an identity guarantee.
The job preserves existing destinations, checks the completed MP4 structure and
stream metadata, then decodes its video and audio before publishing. Verification
checks the requested resolution, rational cadence, frame count and stream timing;
it runs on a cancellable worker and needs FFprobe beside FFmpeg. Publication never
overwrites another file. Cancellation and process failure retain actionable errors;
process watchdogs distinguish stalled work from progressing finalization. A
folder-sync failure after successful publication reports the retained output path
and a durability warning.

## Native replay and recovery limits

Recording journals, immutable assets, audio, video and temporary publication files
share one take-wide disk allowance. Temporary audio mux output counts alongside
the original video until replacement succeeds. Exhaustion stops the affected
capture with an explicit error and retains committed prefixes and original media.
The allowance counts file extents; filesystem allocation overhead and other
applications' disk use remain outside it.

New takes retain received actors and prepared visual channels independently of
the original camera. Custom cameras cannot reveal world data never received from
the server. Recording admits actor animation outside the live camera's animation
frustum; gameplay draw visibility stays independent. Additional animation work
still shares the native evaluation budgets and needs hardware cost measurement.
Block preparation warms incrementally, and native capacity limits or
missing artwork are reported. Weather uses the recorded climate/roof grid.
Prepared HUD and item terrain-visibility publications can lag one rendered frame;
particle, item and block geometry are sampled at the requested capture cadence.

Orbit, follow and other external native cameras use a separately captured world
body, including your skin, even when you recorded in first person. The recorded
POV still hides that body. Takes recorded with hidden first-person body bones need
to be recorded again; switching cameras cannot restore their omitted body pose.
Edited cameras test their near-plane volume against captured animated body and
equipment bounds, using recorded bones, scale and world placement. Intersecting
layers are hidden independently and restore on separation; the camera pose stays
authored. Source and spectated first-person body rules remain separate. Cached
per-bone bounds are conservative: transparent texels, disjoint shapes and triangles
joining different bones can hide a layer earlier than its exact visible surface.
This is not triangle clipping or arbitrary-skin visual parity.

Older takes remain readable but cannot regain omitted visual channels. Use a new
take to capture the expanded coverage, including prepared world nametag artwork
and player/entity classification. World tags retain the eligible received set,
including labels behind the recording camera. The recorded viewer's distance,
server hide rules, settings and native preparation budgets still limit that set;
edited cameras cannot reconstruct omitted labels. Saved label groups use the
edited camera's depth order and billboard orientation.
Interrupted native takes can recover their common committed scene interval into
an unsaved project, after verifying its captured frame grid. Recovery leaves the
original journals unchanged. Recoverable RF64 audio is copied into a separate
finalized file using only complete physical sample frames inside the verified
scene interval. Short audio keeps its real duration and remains unlinked. Durable
bookmarks and unambiguous event observations are restored with source provenance;
legacy combat correlations are not reconstructed. Lost RAM data and uncommitted
tails cannot be restored. Companion failures are reported independently. If
native recovery fails, a decodable POV video can be
recovered separately with an explicit notice. Video decoding and export require
FFmpeg; final export verification also requires FFprobe.
