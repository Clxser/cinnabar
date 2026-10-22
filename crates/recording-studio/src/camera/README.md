# Camera rail motion

## Inspection navigation

Inspection retains the original 8 blocks/second flight and fourfold Shift speed.
Cameras / Views adjusts speed, restores its default, enables optional smooth
translation and levels roll without changing the other pose fields. Left drag
orbits, right drag looks, middle drag pans on a frozen view plane, wheel dollies,
Ctrl/Cmd+wheel changes optical zoom, and right-held wheel adjusts flight speed.
Help and the focused-view hint read the same portable gesture registry as input.

These preferences and motion are session state, not project metadata. Navigation
never seeks or creates undo entries. Apply and named-view saving remain explicit.
Focus loss, text input, hidden viewports and document/seek/source changes discard
momentum and captured pan state. View-only rendering refreshes retain it.

Smooth translation analytically integrates a constant world velocity target per
input interval, with an 80 ms half-life and the existing 100 ms interval cap.
Constant targets have partition-invariant position and velocity within floating
point precision. Rotating input changes the target between intervals; this does
not promise invariance under differently partitioned mouse events. Look remains
immediate. Below 0.00001 blocks/second, remaining travel below 0.000002 blocks
is discarded to stop idle view publications. Pan and wheel changes are immediate.

## Named inspection views

Projects retain named camera views separately from timeline shots. Saving a name,
renaming, deletion and explicit source rebinding are ordinary undoable edits.
Inspection, paging and recall never dirty the project or seek its recorded state.
Recall restores position, yaw, pitch, roll and FOV exactly.

Each view binds to a replay source ID and its known content hash. Relocating hashed
content preserves the binding; replacing it invalidates recall until an explicit
"Bind to current take" edit. Unhashed takes use their captured path conservatively;
metadata cannot detect replacement of an unhashed file at that same path. Normal
source verification still applies. Deleting a source also removes its saved views
in the same undo step. Legacy projects without saved views open with an empty list.
Save As, open and recovery relocate media and captured binding paths independently
through the document's original source base; relocation never rebinds a stale view
to replacement media.

## Recorded subject selection

The recorded-subject browser starts with Players. Search narrows captured names or
entity types using Unicode lowercase substring matching; it never infers a player
from their name or artwork. All entities remains an explicit fallback for older
takes. Search, paging and selecting a result are transient; retargeting an authored
shot remains a separate undoable command. Result actions retain their source,
preview epoch and exact spawn reference so stale frames cannot select a respawn.
Matching indices are cached per immutable actor publication/query/filter; ordinary
UI paints reuse a bounded page. No accent normalization or fuzzy ranking is applied.

## Rail timing

Path shots keep three independent clocks: geometric travel, keyed orientation,
and the optional FOV curve. `Keyed` retains the original keyed position easing.
`Distance` travels evenly within each key interval. `GlobalDistance` uses the
original first-to-last path-key interval to traverse the whole geometric rail,
with a separately saved travel easing. Position-key spacing therefore does not
change global travel speed. Rotation and FOV keep their authored key clocks.
The existing `Hold` easing deliberately jumps at its end; it is not smooth motion.

`Authored` continuity retains existing automatic and explicit handles exactly.
`Smooth` uses a common direction for both handles at an interior key while
retaining each authored handle's length. Automatic handles use a third of their
adjacent chord length. This aligns the one-sided geometric tangent directions;
it can overshoot a key's axis-aligned bounding box and is an explicit geometry
edit. A global distance clock then gives both sides the same world velocity.

A true reversal, cusp, or unaligned authored corner cannot retain nonzero speed
and a continuous velocity. Global travel inserts a local stop at these joins.
Its normalized distance warp uses `6x^3 - 8x^4 + 3x^5` on each half of the stop.
The warp's first and second derivatives are zero at the stop, and are one and
zero at its outer edges. Stop zones have at most 0.025 normalized progress on
either side and shrink near adjacent corners. Zero-length geometry intervals
consume no distance; they retain their independent orientation keys. The whole
rail remains monotone in traveled distance. A stationary rail stays stationary.
This stop detection covers segment joins; arbitrary interior cusps inside one
authored cubic still require an additional key or revised handles.

Trimmed shots retain original geometric support keys and accumulate `path_offset`.
Moving a shot changes its timeline span without changing that offset. Consequently
global easing, corner stops, control geometry, orientation and lens phases do not
restart after a trim or move. Hidden support keys count toward the existing key
budget but are not exposed as editable keys outside the visible trim.

Whole-rail position does not arrive at a geometric key at its orientation timestamp.
Exact XYZ value edits, Add/Update pose key and Apply view therefore explicitly
require switching Travel to Key easing; no pose is silently inserted at the wrong
arrival time. Shape handles still edit geometry. Existing orientation keys remain
editable without moving their spatial points; FOV edits use the independent lens
curve. Inserting a new orientation/pose key requires the same explicit timing switch.

## Numerical and caching contract

Each immutable distance rail compiles 64 arc cells per cubic segment using
five-point Gaussian quadrature. A three-point rule on the same cells provides an
**estimated** length error, exposed by `CompiledCameraPath::accuracy`; this is not
a rigorous bound for arbitrary authored cubics. Inverting a cell uses at most
eight bracketed Newton/bisection iterations, with a requested residual of
`cell_length * 1e-11 + 1e-12` blocks. Exhausting that iteration budget does not
establish that residual. Collinear and stationary cases have explicit handling.
Floating point evaluation and quadrature remain numerical approximations;
runtime and representative difficult-curve acceptance are still required.

The timeline compiles arc tables only when its immutable project changes. The
editor additionally caches the selected shot by document revision, shot identity
and draft identity, so scrubbing and moving an inspection camera do not prepare
new tables. Cached sampling and analytic velocity depend only on source time,
not previous evaluation order. Standalone shot convenience sampling may prepare
a temporary global rail and is intended for metadata edits and isolated callers;
frame adapters and inspectors use their retained compiled rails.

Global overlay geometry searches compiled traveled-distance prefixes for the trim
endpoints and centers its bounded segment window on actual traveled position.
Editable key/handle markers retain their separate timestamp window. Very long
visible rails show a bounded neighborhood rather than dropping the current spatial
segment because its key timestamp is far from the playhead.

## Authored rotation

Select a Free camera or seek to an existing Path key, then choose **Rotate** in
the viewer. The image-space rings edit the renderer's Euler components: yaw uses
world Y, pitch uses yaw-local X, and roll uses the viewing direction. Clicking an
angle opens the existing precise numeric control. These controls retain position,
FOV, rail timing, easing and Bezier handles, including global distance rails.

The ring captures its screen geometry until the original button releases. A drag
previews the authored camera and commits one undo step. Escape, focus loss, a
different take, seek or document change cancel it and restore the prior inspection
view. A successful edit remains on the authored timeline camera. The rings need
an image at least 180 by 116 logical pixels; smaller images retain numeric controls.
Pitch stops at ±90 degrees. Yaw and roll unwrap continuously across ring crossings;
successive pointer samples must differ by less than 180 degrees to identify turn
direction. This is constrained Euler authoring, not an arbitrary quaternion or
world-position rotation manipulator.
