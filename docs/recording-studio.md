# Recording Studio

Recording Studio is experimental. Capture, playback and export need hardware and
long-session validation; requested FPS is not a guarantee of sustained capture.

## Record a take

Open Recording Studio and its settings before recording. Choose Recording FPS
(24, 30, 60, 120 or 240) and Recording size. New preferences use a 1080p limit;
Native keeps the window size. Size limits preserve aspect, never upscale, and use
GPU scaling before readback. Keep the window size fixed during a take.

Hand geometry and HUD/chat capture are separate opt-in native replay channels.
With Record HUD and chat off, new POV video also excludes screen overlays while
leaving the live HUD visible. World labels remain part of the recorded world.
Stop recording and wait for finalization before opening its completed take. Missed capture slots repeat a
frame and are reported; use a smaller output or lower cadence if misses accumulate.
Recording at 120/240 FPS gives slow-motion edits more source frames when the
renderer and encoder can sustain that cadence.

During capture, the status row reports sample and actor preparation CPU time,
pending GPU readbacks, encoder queue depth and stalls. These are capture health
signals, not end-to-end frame-time measurements. Native and scaled recordings
reuse bounded GPU readback buffers, and software live encoding limits its worker
count to leave capacity for the game. Automatic encoding probes NVIDIA support
before falling back to CPU. Sustained capture and the game FPS cost still need
measurement on the target machine.

## Workspace and preview

Settings separates Recording, Preview, Export, Setup and Controls into full-height
pages. Setup checks FFmpeg and FFprobe and actually encodes a private frame to
check the selected format's CPU and NVIDIA encoders. Its CPU fallback action
changes the project's export setting. Recording uses its own admitted preferences.

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
  windows show one side panel or Preview/Tools/Properties tabs, with a persistent
  timeline outside Focus view and separate full-height panel scrolling.
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
- Navigate keeps the picture clear. Players enables small player-selection
  crosshairs, with labels only on hover; entity types come from recorded protocol
  metadata, not their display names. Older takes cannot classify remote subjects;
  Cameras offers an explicit All entities fallback for those takes.
- Path enables rails, keys and Bézier
  handles; drag a key or handle in the view plane and release to commit one undoable
  change. Auto handles restores the original smooth path. Direct spatial editing
  requires one native replay layer with default framing.
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

These additions are source-reviewed and formatted only. Regression source is
included but unrun. Camera transitions, rendered controls, audio synchronization,
range exports and sustained recording performance still need acceptance testing.

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
The job preserves existing destinations, validates the completed MP4's top-level
structure, and publishes without overwriting another file. This is not a full
packet decode check. Cancellation and process failure retain actionable errors;
process watchdogs distinguish stalled work from progressing finalization. A
folder-sync failure after successful publication reports the retained output path
and a durability warning.

## Native replay and recovery limits

New takes retain received actors and prepared visual channels independently of
the original camera. Custom cameras cannot reveal world data never received from
the server. Block preparation warms incrementally, and native capacity limits or
missing artwork are reported. Weather uses the recorded climate/roof grid.
Prepared HUD and item terrain-visibility publications can lag one rendered frame;
particle, item and block geometry are sampled at the requested capture cadence.

Orbit, follow and other external native cameras use a separately captured world
body, including your skin, even when you recorded in first person. The recorded
POV still hides that body. Takes recorded with hidden first-person body bones need
to be recorded again; switching cameras cannot restore their omitted body pose.

Older takes remain readable but cannot regain omitted visual channels. Use a new
take to capture the expanded coverage, including prepared world nametag artwork
and player/entity classification. World tags retain the eligible received set,
including labels behind the recording camera. The recorded viewer's distance,
server hide rules, settings and native preparation budgets still limit that set;
edited cameras cannot reconstruct omitted labels. Saved label groups use the
edited camera's depth order and billboard orientation.
Interrupted POV video can be recovered to
a separate file when decodable media remains; this does not recover incomplete
native scene journals. Recording and export require FFmpeg.
