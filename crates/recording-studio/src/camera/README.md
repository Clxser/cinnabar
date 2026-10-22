# Camera rail motion

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
