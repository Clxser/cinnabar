# Recording Studio

Recording Studio is experimental. Capture, playback and export need hardware and
long-session validation; requested FPS is not a guarantee of sustained capture.

## Record a take

Open Recording Studio and its settings before recording. Choose Recording FPS
(24, 30, 60, 120 or 240) and Recording size. New preferences use a 1080p limit;
Native keeps the window size. Size limits preserve aspect, never upscale, and use
GPU scaling before readback. Keep the window size fixed during a take.

Hand geometry and HUD/chat capture are separate opt-in native replay channels.
POV video still contains the final displayed image. Stop recording and wait for
finalization before opening its completed take. Missed capture slots repeat a
frame and are reported; use a smaller output or lower cadence if misses accumulate.
Recording at 120/240 FPS gives slow-motion edits more source frames when the
renderer and encoder can sustain that cadence.

## Edit

- Scroll over timeline tracks to move vertically. Shift+wheel, horizontal wheel,
  or wheel over the ruler pans through time. Ctrl+wheel zooms around the pointer.
- Use Fit for the whole project, Playhead to reveal the current frame, or drag the
  bottom overview to move the visible window. Follow tracks the playhead while playing.
- Drag a clip to move it; drag an actual visible edge to trim it. A drag commits
  once on release and can be undone. Ripple is an explicit timeline setting.
- On narrow windows, switch between Preview, Media and Inspector. Short windows
  also scroll the workspace so the timeline and preview remain reachable.
- The clip inspector accepts exact Position, Source In and Source Out seconds.
  Source fields preserve the explicit Position; Apply commits, Cancel discards.
- Add speed keys at the playhead, choose incoming easing, or browse keys with
  Prev/Next. Video, linked game audio and generated event edits share this clock.
  Ramp audio changes pitch; constant-speed audio keeps its existing tempo path.
- Camera paths have visible keys, smoothing/easing controls and numeric values.
  Looks expand into ordinary editable effect curves. Individual color, grain,
  chromatic, soft-focus and sharpening effects can also be authored directly.
- Import audio/video/images or relink a selected source from Media. Output size,
  FPS, codec, encoder, audio and destination are available in settings.

## Native replay and recovery limits

New takes retain received actors and prepared visual channels independently of
the original camera. Custom cameras cannot reveal world data never received from
the server. Block preparation warms incrementally, and native capacity limits or
missing artwork are reported. Weather uses the recorded climate/roof grid.
Prepared HUD and item terrain-visibility publications can lag one rendered frame;
particle, item and block geometry are sampled at the requested capture cadence.

Older takes remain readable but cannot regain omitted visual channels. Use a new
take to capture the expanded coverage. Interrupted POV video can be recovered to
a separate file when decodable media remains; this does not recover incomplete
native scene journals. Recording and export require FFmpeg.
