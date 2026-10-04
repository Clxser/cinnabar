# Biome tint sampling evidence


The biome cache has seven points per axis, spaced four blocks apart and
covering offsets -12 through +12 from its origin. Each point counts 27 biome
samples at offsets -4, 0, and +4 in X, Y, and Z. It retains the four most frequent
biome IDs, dividing their counts by all 27 samples without renormalizing
discarded IDs.

An integer block-position query finds the eight nearest points from 27
candidates. Each weight is `1 / (distance + epsilon)`, normalized by the sum
of those weights. Epsilon is **1.1920928955078125e-7**, exactly `f32::EPSILON`.
Equal-distance candidates retain X/Y/Z traversal order. Z indexing adds one
lattice step relative to the loop variable, making candidates symmetric on
all axes. Cell division truncates toward zero relative to the cache origin.

The origin is the render chunk's minimum block position plus eight on each
axis. The biome snapshot uses four-block spacing within radius 16. Entries
start at zero, and an absent chunk leaves a zero biome ID. Snapshot and cache
selection depend on a capability whose mapping to graphics settings remains
unresolved.

Grass has a direct-color path when the cache is disabled. Its cached path
samples at an integer block position with tint method 5; this does not establish
every grass, foliage, or water tessellation route. Tint strategies 1–4 use
foliage palette callbacks, 5 selects grass, and 6 selects water. Grass and water
use the lattice under the same capability test, otherwise reading per-column
colors. All four foliage kinds use the lattice unless seasonal tinting applies.
The foliage fallback averages an offset list whose shape remains unresolved.

Vines query tint once at their integer block position before writing vertex
colors. A separate grass path averages 25 horizontal samples spaced four blocks
apart at the supplied Y. It is distinct from the lattice cache; the graphics
mode selecting it remains unresolved.

## Cinnabar change and limits

The render shader already used a provisional horizontal four-block lattice;
`biome.rs`'s radius-one helper only drove CPU diagnostics. Changing its radius
alone would not change the screenshot. The shader used separable linear
weights, ignored vertical neighbours, and clamped absent neighbour samples to
its own edge. Model tint queries also varied over model geometry.

The new record caches the 3D lattice counts, carries 27 neighbour identities,
uses biome zero for absent samples, and invalidates vertical and diagonal mesh
consumers on arrival, replacement or eviction. Query weights are generated once
from Rust for the 343 signed within-cell integer positions. Uniform records
skip the lattice allocation. A nine-point-per-axis scratch grid reduces packed
biome lookups from 9,261 to 729 per nonuniform record. Model vertices share their
own block's tint query. Foliage kinds select their palettes inside the average.

**Incomplete:** this ports the verified lattice cache, not a proven universal
rule for every vanilla graphics mode and tint type. Top-four ties involving
more than four equally frequent biome IDs, all per-corner tessellation routes, the
capability-to-setting mapping, and vanilla neighbour-arrival invalidation still
need reference evidence. The owner's screenshot has no retained biome records;
its live hard edge has not been reproduced or conclusively attributed.

The PNG gallery is a CPU palette preview with synthetic artwork, not a GPU
frame or native acceptance witness. The complete-halo before preview can already
show a gradient; it cannot prove that this change fixes the owner's live seam.
No live server connection is needed or used by these tests.
