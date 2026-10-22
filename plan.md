# Cinnabar roadmap

Implementation status and acceptance requirements live here. Change history is
in [generated release notes and merged PRs](CHANGELOG.md). The
[completion ledger](docs/evidence/phases-2-5-completion-ledger.md) records acceptance
evidence; the [workspace map](docs/architecture/workspace.md) explains code ownership.

## Maintaining this plan

- Update the existing section when scope, remaining work or acceptance changes.
  Say what exists before describing a confirmed behavior gap or missing verification.
  Keep entries short; detailed behavior belongs with its code or existing reference docs.
- Keep provisional behavior and missing evidence explicitly incomplete. A merged PR,
  passing unit test or refactor does not by itself close a parity or performance gate.
- Keep each phase requirement ID consistent with the completion ledger. The architecture
  check requires each ID exactly once in its owning phase below.
- Write a brief imperative PR title and apply one category label for release history.
  Do not append a PR diary here or maintain a second handwritten changelog.
- Keep internal research and agent reports in the private maintainer checkout described
  in [AGENTS.md](AGENTS.md). Earlier plan revisions remain in Git history.

## Current priorities

1. Identify and resolve confirmed terrain, actor, movement and UI behavior gaps with version-matched evidence.
2. Qualify frame pacing, streaming, joining and retained memory on the reference hardware.
   Windows VSync queue depth uses coarse work estimates to try one queued frame and
   observed acquisition cadence to restore two. Failed probes back off exponentially;
   missing GPU samples leave the cadence policy running.
   Displayed cadence, input latency and surface-reconfiguration
   stalls still need hardware qualification; this closes no acceptance gate.
3. Verify complete session and input paths across supported platforms.
4. Simplify existing subsystem ownership without adding unnecessary crates or adapter layers.

**The unchecked boxes below describe full acceptance requirements, not absent features.**
Terrain rendering, movement, actors, chat, HUD, inventories and forms already have
implementations. These criteria come from the earlier plan and have not had a complete
status audit against current code. Check the implementation and later evidence before
treating an inherited entry as new work. Missing acceptance evidence means a feature
is not fully qualified; it does not mean the feature is unimplemented. This cleanup
closes no gate and provides no new runtime or performance evidence.

## Phase 0 â€” End-to-end acceptance

- [ ] Complete a sustained streamed-world acceptance run: join, move at speed, and
  break/place blocks to exercise live remeshing. Resolve outstanding command/protocol
  conformance findings, classify every decode error, and qualify frame and mutation
  latency against the current [live-testing requirements](docs/agents/live-testing.md).
  Use the current headless client workflow for scripted tests; keep captures local.
  Dimension definitions use an exclusive maximum Y and retain complete sub-chunks;
  live custom-dimension transfer acceptance remains open.

## Phase 1 â€” Core service

The Go core and local stream boundary are implemented. Keep authentication,
encryption and upstream transports with the core, and client behavior with Rust.
Changes still need current protocol conformance, bounded framing and lifecycle coverage.
The pinned target comes from [the target manifest](assets/bedrock-target.json).
Invalid saved-token contents recover through device sign-in, preserving the cache
until authentication succeeds. Filesystem failures retain credential-free operation
and numeric OS diagnostics. Live Windows recovery acceptance remains open.

## Phase 2 â€” World rendering

Implemented: [terrain meshing](crates/meshing/src/lib.rs),
[packed biome storage](crates/world/src/biome.rs),
[biome tint preparation](crates/meshing/src/biome.rs), and
[cloud](crates/render/src/cloud_render.rs) and [weather](crates/render/src/weather_render.rs)
rendering. The criteria below concern complete target coverage, native comparisons
and resource qualification; they do not imply these systems need to be written.

### Terrain, materials and biome acceptance

- [ ] Close the opaque texture slice with matched native views, full-view remesh
  qualification, and fresh combined memory and steady-CPU measurements.
  Furnace face slots follow cardinal direction and lit state; native visual comparison
  and runtime replacement-pack acceptance remain open.
- [ ] Close cutout-cube and leaf visual acceptance. Preserve independent geometry,
  occlusion and cave-connectivity semantics, packed quads and coverage-preserving mips.
- [ ] `P2.5-NATIVE-BIOME` â€” Qualify existing biome palettes and grass/foliage/water tinting
  against native output while preserving packed storage and the shared quad contract.
- [ ] Confirm or replace provisional biome blending using version-matched radius and
  kernel evidence. Cover abrupt boundaries, special foliage, custom-biome fallback,
  missing neighbours, replacement, teleport/eviction, GPU output and performance.

### Block-family and gallery acceptance

- [ ] Qualify static/non-cube models, blend/water and flipbooks, including native
  presentation and animation. Identify unsupported states before adding implementation.
- [ ] Close straw-bed native-gallery and replacement-pack acceptance. Both halves
  use pack geometry in all four directions; sleeping and picking remain unqualified.
  Beacon base/core/shell compilation remains provisional: the base's 0.1-pixel
  bottom offset rounds to the model grid; matched world and replacement-pack
  captures, carried-item presentation and finer coordinate precision remain open.
- [ ] Qualify compact templates for slabs/stairs, thin attached faces, paired doors,
  trapdoors, connected panes/fences/gates and static chest/sign models. Check existing
  coverage before implementing a missing case. Keep conservative culling and connectivity
  until exact face-coverage behavior is verified separately.
- [ ] Close paired-door presentation and interaction acceptance, including lower/upper
  state resolution, cross-subchunk neighbours, missing pairs, highlights, cracks,
  directional replacement-pack artwork and native-server toggle/picking input.
- [ ] Close barrel/trapdoor visual acceptance, including facing/open states, grain,
  carried items, edge UVs, replacement materials and different texture resolutions.
- [ ] Close chiseled-bookshelf native-gallery acceptance with two consecutive exact
  GPU-completed model-stream witnesses.
- [ ] Close resin-clump native-gallery acceptance with two consecutive exact
  GPU-completed model-stream witnesses.
- [ ] Close selector-alias opaque-cube acceptance for axes, TNT and deprecated states,
  with matched views and two consecutive exact GPU-completed cube-stream witnesses.
- [ ] Close cactus overview, stacked, grazing and inset-top presentation acceptance,
  with stable exact model-stream witnesses and zero contamination counters.
- [ ] Close cake presentation at representative bite states, with stable exact
  model-stream witnesses and zero contamination counters.
- [ ] Close candle-cake flame-particle and native-world presentation acceptance;
  isolated composite geometry and texture fixtures do not establish the full result.
- [ ] Close farmland presentation at representative moisture states, with matched
  native views, stable exact model-stream witnesses and zero contamination counters.
- [ ] Close crop comparison, sub-unit sprite geometry, atlas-edge UV calibration and
  torchflower positional variation. Earlier bounded artwork fixes do not close this scope.
- [ ] Verify nether-wart geometry and selector acceptance independently of other crops.
- [ ] Close all fixed slab/stair gallery poses with exact GPU witnesses, matched native
  views and a clean mutation-to-visible/performance run. Isolate backend/presentation
  faults from geometry; do not mask them with speculative renderer changes.
- [ ] Close vine attachment-mask, neighbour, connectivity and texture/UV acceptance.
  Adjudicate glow lichen and sculk vein separately rather than assuming identical rules.
- [ ] Close the exhaustive residual-family inventory for the pinned target. Every
  non-air state needs an authoritative accepted visual; provisional fallback does not count.
  Require zero diagnostic/provisional-fallback counters and current resource-cost evidence.
- [ ] Close coral-fan native placement and rendered acceptance for floor/wall orientations
  and replacement-pack artwork.
- [ ] Close tripwire-hook native placement and rendered acceptance across facing,
  attachment and signal states.
- [ ] Close shelf native-gallery, pack-pixel fidelity and live replacement-pack acceptance.
- [ ] Close the static-sign gallery and native visual acceptance.
- [ ] Establish exact standing/hanging sign board, post and support geometry, UVs and
  GPU/native evidence. Provisional dimensions are not final geometry parity.
- [ ] Complete every exact-state GPU gallery page for the pinned inventory, with exact
  palette readback, two consecutive completed frames and matched native views. Derive
  page/state totals from the current inventory; support fixtures do not count as targets.
- [ ] Verify the separately versioned block-entity inventory and renderer manifest,
  including chunk NBT, live updates, required variants and explicit GPU/no-draw evidence.
- [ ] Close every registered block-entity renderer route, gallery builder, required
  NBT variant and strict-final witness without deferrals.
- [ ] Qualify the provisional block-entity carrier/render pass across its supported
  families, updates, animation and item contents; implementation alone closes no gate.
  Shulker lids still need animated collision bounds and standing-on-open-lid acceptance;
  accepting server actor displacement alone does not complete this behavior.
  Local actor displacement also needs matched interpolation-timing acceptance.
- [ ] Close protocol/content integration findings and required upstream fixes through
  the current PR, review and acceptance workflow. Follow the current branch policy.
- [ ] Verify invisible-block presentation, including conditional camera-facing marker
  sprites and their display/input conditions; invisible terrain alone does not close acceptance.

### Lighting, atmosphere and publication

- [ ] `P2.7-ATMOSPHERE` â€” Complete lighting, day/night, sky, fog, weather and cloud
  acceptance, together with the remaining terrain publication and remesh qualification.
- [ ] Classify and remove moving bands/static under camera motion using coherent
  resident, frustum, submitted and GPU-completed identities. Change culling only when
  evidence proves a missing draw.
- [ ] Run identical-scene release FIFO/Immediate presentation comparisons and fix the
  demonstrated source of the artifact.
- [ ] `P2-CHUNK-PUBLICATION` â€” Qualify initial publication, steady streaming and full-view
  remesh against current frame/join/streaming budgets. Attribute decode, light readiness,
  queue wait, worker time, upload, submission and present latency independently.
  Ordinary meshing now rechecks higher-priority deferred revisions within a bounded
  poll budget, with backoff while neighbours or light remain unavailable. Hardware
  dirty-to-visible latency qualification remains open.
  Sparse startup can complete without a publisher update after terrain decodes, stream
  and render work drain, and a later GPU frame completes. Empty-world initialization is
  verified; complete server-form and gameplay acceptance remains open.
- [ ] Close mesh-neighbour readiness acceptance with slow-delivery and seam coverage.
  Distinguish eligible horizontal columns from the provisional full-neighbour/quiet-time
  fallback; its timeout is not established native behavior.
- [ ] Close publisher membership/retention acceptance using FIFO-committed announcements
  and epoch identity. Keep raw-radius classification and confirmed player interest
  separate; no single enumerable Euclidean disk defines all server cohorts.
- [ ] Recheck reported sun/moon background rectangles and filter-edge artifacts using
  decoded pinned pixels and mip-edge tests across bright/dark skies and every moon
  phase. Fix any reproduced defects before closing their acceptance record.
- [ ] Close fresh release/native sun and moon comparisons, including horizon and
  filter-edge cases, before completing the visible-defect gate.
- [ ] Close precipitation geometry, wrapping, UV, occlusion, density and smoothing
  acceptance. Native wind/permutation, turbulence, block-light tint and view flags
  remain unqualified where the current implementation is provisional.
- [ ] Close volumetric cloud/layer acceptance from above, below, within and at grazing
  angles, preserving world anchoring, weather/fog fades and bounded GPU work.
- [ ] Recheck cloud scale, thickness, silhouette, shading, motion and seam differences
  with a fresh accepted native comparison gallery; resolve reproduced discrepancies.
  Deterministic mesher tests do not establish the rendered comparison.
- [ ] Confirm cloud texture scale, height, face shading, drift, day/weather/sunrise colour
  and distance-fade behavior for the pinned target. Historical profile approximations
  remain incomplete until calibrated against that target.
- [ ] Independently review and live-verify the final finite-cloud result.

The [family inventory](docs/phase-2-family-inventory.md) is a historical baseline;
regenerate target inventories from the current manifest before qualifying them.
See the [rendering reference](docs/reference/rendering.md) and
[chunk-pipeline architecture](docs/architecture/chunk-pipeline.md) for detailed contracts.

## Phase 3 â€” Movement and the local player

Implemented: [movement, collision and prediction](crates/sim/src/lib.rs),
[semantic input](crates/input/src/lib.rs) and the
[client interaction adapters](crates/client-interaction/src/lib.rs). Open acceptance
is a broader claim than the presence of these features.

- [ ] `P3-MOVEMENT` â€” Close movement, collision, prediction/correction and live-server
  acceptance for the pinned target. A server-compatible run is not full native parity.
- [ ] `P3.4-INPUT-CAMERA` â€” Close semantic input, camera and device-specific behavior.
  Touch remains open even when an owner-deprioritized touch witness does not gate a
  narrower scenario verdict.

View Bobbing offers Off, Full and Minimal. Minimal retains first-person hand bobbing
with a steady movement camera in both animation modes; hurt and death effects remain
independent. In-world rendered acceptance evidence for this client option remains outstanding.

Historical movement acceptance scope includes sprint/double-tap/toggle timing, forced
sneak/crawl, mode entry/exit, flight, swimming/gliding, liquid correction and scaffolding
descent. Reconcile each case with current code and evidence before calling it a missing
implementation.
Keep raw, analogue and processed input semantics distinct. Mounted controls need
attributable vehicle mapping, constants, state and actual prediction before claiming
client-predicted vehicle behavior. Riding alone does not establish that capability.

Correction-sync acceptance still includes predicted flag coverage, scaled/horizontal
poses and bounds, history loss, replay authority, send phase and live-server behavior.
Animation-only flags must remain separate from gameplay and correction authority.
Snapped corrections and teleports keep the local input tick when the server stamp is
unretained or ahead; acceptance on servers that stamp their own clock is outstanding.
Server sneak updates preserve local hold/toggle intent, and the next completed input
corrects a conflicting stance through start/stop actions retained across tickless frames
and replay. Live-server pickup and remote-player stance acceptance remains outstanding.
Near-contact collision fixes do not establish complete collision-engine parity.

See [gameplay/session ownership](docs/architecture/gameplay-session.md),
[effective movement speed](docs/reference/effective-movement-speed.md) and
[camera behavior](docs/reference/camera.md).

## Phase 4 â€” Entities and equipment

Implemented: [actor rigs, Molang, controllers, attachables and pose evaluation](crates/actor-animation/src/lib.rs),
with [client presentation](crates/client-presentation/src/lib.rs) and
[rendering](crates/render/src/lib.rs). The open gates require complete behavioral
and rendered qualification, not initial implementation of those systems.
Frame sampling retains matching expression, layer and composition results and evaluates
remote body batches on the caller and available shared workers with ordered budget
reservations. Equipment placement reuses unchanged owner and attachment inputs. Rig plans
retain clip masks and name indices sized to each skeleton. Rig plans and prepared skins
share cape and cosmetic attachment bindings. Hardware crowd captures and full temporal/visual
qualification remain open.

Pure numeric Molang expressions use bound f32 lane programs with load fusion;
mixed values and effects retain the generic evaluator, including exact budget tails.
Full rig history and rendered qualification remain part of the rig item below.
Body and custom-skin composition use shared parent-first programs and caller-owned
output, including attachable root modes and legacy attacks. Alternate render
geometries and skin layers retain recursive composition. Rendered and live
performance qualification remains open.

Body-slot equipment retains its replicated identity and selects authored attachable
geometry, artwork and owner-root placement. Startup harness definitions are completed
from tagged items and present pack resources. Resource-only session overrides reuse
those definitions while preserving authored item conditions. Expression roots retain
the worn slot through owner placement; worn equipment counts exclude hands.
Multi-pass body equipment and matched native visual qualification remain incomplete.

- [ ] `P4.3-RIGS` â€” Close bounded asset admission, Molang/controllers, protocol-driven
  poses, skeletal rendering and distinct tick/frame interpolation with native evidence.
- [ ] `P4.4-LIVE-ACTOR` â€” Close spawn, movement, rotation and teleport ground-contact
  acceptance, including correct player-origin conversion and frame interpolation.
  Keep visual eye height and the protocol movement offset as separate owned facts.
  Player deaths hide after their brief sequence while retaining the actor; later
  server flags control visibility through revival. Original Zeqa acceptance remains open.
- [ ] `P4.5-ITEM-ACTIONS` â€” Close held items, actions, dropped items and viewmodel behavior,
  including remote/local playback and session lifecycle.
  Held heads reuse the placed-head meshes and textures; their native hand pose, scale,
  inventory preview and resource-pack geometry acceptance remain incomplete.
- [ ] Close pickup sound and copied-item flight across immediate server removal with
  a version-matched rendered comparison.
- [ ] Close custom block-item texture resolution, non-cube shapes, tinting, animation
  and matched inventory/held-item presentation. Shared cube face sheets are bounded coverage.

Continuous cosmetic cloth uses shared vertices, fixed-step constraints, world-space
inertia and contacts derived from the drawn body and worn armor. Cloth pins and
mounts retract toward the bare torso and expand to clear equipped chest geometry.
Vertical follow and damping restrain jump and landing motion, and local cloth keeps
advancing while first-person visibility hides the body. View changes transfer the drape between
body pose spaces. Self-collision and world collision remain open.
Paid cosmetic selections use session ownership from the API database and withdraw
on disconnect or revocation. Unowned wardrobe tiles are dimmed with a lock badge
and excluded from pointer and keyboard wear selection; production rollout
qualification remains open.

Equipped cosmetic emitters also run in visible menu and HUD paper dolls using the
shared particle engine and attachment poses. Catalog-wide, in-world interaction
and performance qualification for these previews remains open.

Player name tags use Java's standing player height before pose offsets, independent of
server collision-box changes. Other actors retain their published label height.
Name-tag budgets admit on-screen bounds before their caps, favor nearer labels and
keep cosmetic labels complete. Atlas text
retains formatting and line slots, with guarded cell edges and
repacking before UV publication. In-world visual qualification remains open.
Name tag colours compose after Enhanced and shader-pack world effects with the scene's
depth, and translucent plates use over-compositing for scene alpha. Coplanar shadows
and glyphs share the anchor depth to avoid rounding speckle. Local cosmetic
colour selection uses actor identity. In-world visual qualification remains open.
Upright player name tags keep Java's standing height before pose offsets; swimming,
crawling and gliding players use their live height. Other actors retain their published
label height. Tag eligibility uses always-show metadata or the shared crosshair pick.
Camera distance controls server render limits and score range; passenger mounts keep
their name without a score line. Depth testing honors sneaking and the nameplate flag.
Matched rendered-frame qualification remains open.

Remaining qualification includes attachables and custom item models, action/use timing,
projectile interpolation and shake, sleep/mount poses, name tags (including over
oversized server cosmetics), capes, shadows,
particles and equipment materials. Preserve existing accepted Java animation/cape/name-tag
and combat-visual deviations described in [AGENTS.md](AGENTS.md).

See [actor animation clocks](docs/reference/actor-animation-clocks.md),
[projectile rendering](docs/projectile-rendering.md) and
[presentation ownership](docs/architecture/client-presentation.md).

## Phase 5 â€” Interaction, inventory and UI

Implemented: [chat, HUD and scoreboard state](crates/ui/src/lib.rs),
[UI runtime and server forms](crates/client-ui/src/ui_runtime.rs),
[inventory prediction and crafting](crates/inventory/src/lib.rs), and
[menus and settings](crates/client-shell/src/menu.rs). The boxes track full acceptance
across these existing features; they are not a list of missing screens or systems.
Outgoing chat uses the session's login name and XUID. Server-authored actor names
remain available for local presentation and text resolution.
Gameplay mouse capture resumes after focus or occlusion returns when it was active
before switching away. Screen ownership and focus-loss input clearing remain in effect;
OS Alt-Tab acceptance on macOS, Windows and Linux remains incomplete.

- [ ] `P5.1-UI` â€” Close pack-driven UI, bounded text/draw work, formatting, scaling,
  safe areas, focus/navigation and keyboard/mouse/controller/touch acceptance. Prove
  stable retained resources and correct supported DPI/aspect behavior. Labels share primary/backup
  face selection across measurement and painting; broader text acceptance remains open.
  Rounded JSON-UI chrome supports bounded analytic box shadows with cached geometry;
  full menu visual and hardware frame-work acceptance remains open.
  Client-owned panel collections support bounded four-column card grids with
  item-scoped expressions, vector artwork and one-time guest requests to reveal filtered
  items without rebuilding the catalog; hardware frame-work acceptance remains open.
  The desktop mouse Back button uses existing cancellation in menus, containers,
  chat popups, forms, signs and emotes; binding capture still accepts the button.
  Cross-platform device acceptance remains incomplete.
  stable retained resources and correct supported DPI/aspect behavior. Tiled images
  retain full coverage with bounded repeat geometry across cropped regions and sliced
  borders; large collection admission and hardware frame-work acceptance remain incomplete.
  stable retained resources and correct supported DPI/aspect behavior. Form JSON
  container parsing and cleanup use heap stacks and accept line/block comments.
  Form display strings retain ordinary long literal and translated text separately
  from chat byte budgets. Structured text keeps its component/depth validation limits;
  extreme structured-text and live rendered-form acceptance remain open.
  The per-form expansion budget (`MAX_FORM_TEXT_EXPANSION_BYTES`), shared by all display strings
  and their nested arguments, is an unverified parity bound.
  Rendering text beyond `MAX_TEXT_BYTES` remains an unverified parity gap.
  Tiled images retain full coverage with bounded repeat geometry across cropped
  regions and sliced borders; large collection admission and hardware frame-work
  acceptance remain incomplete.
- [ ] `P5.2-HUD` â€” Close received text, titles, actionbar, toasts, survival/status HUD
  and lifecycle behavior without leaking UI-owned input into movement.
- [ ] `P5.3-CHAT` â€” Close history/autocomplete, formatting, bounded UTF-8 editing,
  clipboard, submission prefixes, command editing, rate limits and session/FIFO routing.
  Verify third-party receive/send and disconnect behavior.
- [ ] `P5.4-SCOREBOARD` â€” Close objective/score and boss-bar create/update/remove,
  ordering, display positions, styles and stacking alongside titles/actionbar.
- [ ] `P5.5-INTERACTION-COMBAT-INVENTORY` â€” Close targeting, mining/cracks, placement/use,
  combat, hotbar selection, stack/network-ID reconciliation and container/crafting
  prediction, acknowledgements, rollback and stale-session behavior.
  Native block requirements now cover every catalog identifier, including no-tool
  beds; recipe tags share the pinned native catalog. Custom block mining retains
  default, disabled and item-specific component values, including state overrides.
  Discarded mining data stays unresolved, and item-tag filters compile during
  session preparation for allocation-free evaluation.
  Mining uncertainty leaves known physics and local placement prediction usable.
  Mining states use separate storage and admission, preserving the physical state budget.
  Complex item-tag scripts and live custom-block mining acceptance remain incomplete.
  Sword/shears speed rules,
  effective item durability and live mining acceptance remain incomplete.
  Trading screens admit server offers and support purchases with manually placed inputs.
  Automatic ingredient filling, trader experience, restock and demand-price transitions,
  classic preview rich tooltips, and full item/container acceptance across trading layouts
  remain incomplete.
  Block-item purchases await an authoritative result preview to retain its block identity.
  Furnace quick moves retain admitted fuel facts for canonical-empty item descriptors;
  explicit server fuel declarations keep priority. Named station responses retain
  accepted stacks at their window indices. Broader custom-fuel and matched server
  acceptance remain incomplete.
- [ ] `P5.6-FORMS` â€” Close modal/menu/custom form navigation, validation, cancellation
  and responses, including server-pack definitions, structured text, custom controls,
  scaling and image/layout behavior against matched native views. Overlapping requests
  keep the active form and answer busy, including reused IDs.
  Busy cancellations retain every request through transport backpressure.
  Custom forms retain control locks and publish enabled bindings for the current
  game phase without removing locked response values; device acceptance remains open.
  Server settings retain drafts across tabs and submit their final values when
  settings close. Embedding their controls within the settings panel instead of a
  separate form remains incomplete.
  NPC dialogue actions expose command/URL labels and pointer hit targets through
  their JSON-UI collection. Actor portraits, matched acceptance for larger action
  lists and touch/console behavior remain incomplete.
  URL images use the UI texture-file allowance; aggregate encoded-image cache bounds
  and remaining image admission and download-deadline parity remain incomplete.
- [ ] `P5.7-PARITY-PERF` â€” Qualify combined chat, scoreboard, boss bars, inventory and
  forms at supported scales/aspects and devices, with bounded memory and stable frame work.
- [ ] `P5.8-SETTINGS` â€” Close menu, control, video and persistence acceptance. Desktop
  wiring alone does not establish native parity; language-specific scale dialogs,
  safe-zone handling, service-controlled spacing and touch/console behavior remain open.
  Social â†’ Chat exposes the existing chat preferences, shared opacity, duration and
  position controls, and one reset for all displayed Chat settings. Chat uses the bundled
  Cinnangles Sans font without a typeface selector; font sizing applies directly and
  obsolete saved typeface values are ignored. Emote filtering, narration and mention
  highlighting remain unavailable and their controls say they store preferences only.
  Offline rendered fixtures cover the panel, values, palettes and reset confirmation;
  live input acceptance remains incomplete. Font size 5â€“20/default 10 and spacing
  0â€“100/default 0 remain provisional host ranges, without confirmed native defaults.
  Social â†’ Discord now exposes the Rich Presence master switch plus persisted privacy,
  party-size, elapsed-time and join-request controls; each is applied to the Discord
  activity payload and the panel has its own reset. Live Discord desktop acceptance and
  native settings parity remain incomplete.

The inherited interaction acceptance scope also includes full item identity and use phases,
replacement/deferred releases, mounted inventory, sign filtering/length/structured text,
native emote admission/cancellation/timing, overlapping dimension/respawn loading screens,
and server-authored chat hover/input behavior. Respawn readiness retires the death route
and pending retries independently of a later health refresh, in UI packet order; a later
authoritative life-to-death transition opens a fresh route. Regression coverage does not
close live server, loading-screen or other-device acceptance. Keep native input state
independent of custom emote playback and presentation-only changes.

Chat message Send controls resolve command text from an overlapping sibling editor.
Missing, disabled or ambiguous message editors cannot submit the main chat draft.
Live acceptance of name clicks, inventory opening, typing and history together remains open.

Ordered desktop interactions still need live input and overload-timing acceptance,
global camera-sampling qualification, other input modes and device-specific chords.
The bounded burst/neutrality recovery policy remains provisional. Per-frame admission
regressions do not close the surrounding input, movement or interaction gates.

### Held block placement

Held placement still needs confirmation of direction/face/intercept retention, repeat
timing and reset behavior across slot/item changes, unavailable selection and deferred
presses. Preserve the explicit incomplete scope in the
[held-placement reference](docs/reference/held-block-placement.md).

See [inventory ownership](docs/architecture/inventory.md),
[launcher/UI ownership](docs/architecture/launcher-ui.md),
[OreUI](docs/oreui.md) and [desktop settings evidence](docs/evidence/desktop-video-settings.md).

## Phase 6 â€” Online product surface

Online services are implemented. Startup requires login facts and PlayerSpawn;
radius replies remain ordered play updates without holding startup. Complete
acceptance still covers authenticated
server/Realm/friend-world joins, transfers, reconnect/disconnect, account switching,
presence, invites and required-pack failures. Preserve transport/session identity
and keep actual account details out of captures and reports published with PRs.
Core transfer/disconnect terminals can abandon unfinished join preparation; ordinary
preparation and presentation readiness remain required for a successful join. Live
server latency impact and the complete transfer acceptance matrix remain unqualified.

Server pack acquisition, handoff and stack composition no longer impose fixed archive-count
or compressed-byte quotas. Saved server lists retain every imported entry. Declared sizes,
identity, digests and framing remain validated. Experience selections preserve service errors
and wait for the account core result. Complete featured-server acceptance remains open:
large-pack preparation can be slow or fail, and sparse startup terrain can leave loading open.
Server language loading retains every readable English and selected-locale layer without fixed
table, entry or process quotas, and folds inserted keys to ASCII lowercase. Unreadable sources
and allocation failures are diagnosed without discarding other definitions. Complete pack-content
acceptance remains open. Local imports, decompression, UI/text expansion and other admission limits
or compressed-byte quotas. Safe member paths have no added byte cutoff; server manifests
and default member reads retain complete declared content. Manifest subpack lists have no separate
row-count cutoff; server and local loading share the existing field validation and device-tier
selection. Native runtime and device acceptance for larger subpack lists remain open.
Explicit subscriber limits and large-content live and performance acceptance remain open.
Saved server lists retain every
imported entry. Declared sizes, identity, digests and framing remain validated. Experience
selections preserve service errors and wait for the account core result. Complete
featured-server, menu and gameplay qualification remains open, including large-pack
preparation and broader sparse-startup coverage.
Server handoff keeps the advertised UUID and version for stack selection when a valid
archive's manifest names another identity; local imports and library selections still match
exactly. The matching Gophertunnel transfer change and live acceptance remain incomplete.
Local imports, decompression, UI/text expansion, language memory and other admission limits
still need version-matched verification; the limit inventory closes no whole-client parity gate.

Cosmetics require live authenticated multi-account acceptance, catalog/motion envelope
coverage, supported slot/ownership enforcement, player-preview acceptance and hardware
qualification. Payments and moderation remain incomplete; rollout must respect API/client
compatibility. Existing catalog content is owned by the API repository.

## Phase 7 â€” Local worlds

Local worlds use the Go server integration. Qualify creation, offline play, reload,
import, persistence and hosting/session teardown. The accepted local-world scope does
not establish full vanilla world-generation parity.

See [local worlds](docs/local-worlds.md).

## Phase 8 â€” Audio, polish and packaging

Qualify audio admission, spatial playback, music and lifecycle behavior on supported
platforms. Missing source music/ambience requires lawful local input or the documented
fallback; do not bundle game assets to fill gaps.

Complete clean-install, first-run asset preparation, update/install recovery, startup
and crash diagnostics, packaging and platform acceptance. Required carriers must pass
validation; retain only the explicitly permitted diagnostic/optional fallbacks.

Windows runtime executables use `asInvoker` manifests and allow both normal and elevated
launches. Native Windows startup, update and crash-restart acceptance remains incomplete.

## Experimental hosts and extensions

- Browser/Android implementation and CI checks do not close their complete join, pack,
  transfer/reconnect, input/UI, audio, memory or hardware-performance acceptance matrices.
  Built-in shaders avoid forwarding matrix parameters between functions to prevent an Adreno
  Vulkan compiler crash. Startup is confirmed on an affected Adreno 660 device; broader Android
  rendering acceptance remains incomplete.
  Keep browser gesture activation and background work within the host's event-loop rules.
  Touch-capable hosts expose persisted look sensitivity and control guidance. The sensitivity curve is provisional; native touch layouts, remapping and customization remain open.
  Settings supports touch dragging of its content columns and scrollbars; swipes cancel pending row taps. Native gesture calibration and the remaining menu/form touch routes are still open.
  Launcher text fields request platform IME input, and the People drawer filters the current friend list by gamertag. Global people lookup, gameplay text entry and full device IME acceptance remain open.
- Local mods and server experiences still need runtime/input isolation, reload/reorder/
  removal, consent, custom screen transitions and media qualification. Unmeasured shared
  blur, element blur, route exits and current-world visual integration remain incomplete.
- Native extension UI and HUD APIs (retained modules, profile fields, text plates,
  compass markers, totem annotations and bounded camera overrides) still need
  displayed-frame pacing and complete rendering acceptance. Packet extensions now
  negotiate bounded detached-view and passive camera effects through the shared
  camera owners; interactive movement, perspective release, teleport and hurt-view
  acceptance remain unverified in a displayed client. Panel previews now
  select owner-scoped HUD cards, including preview-only cards, and share retained
  gameplay artwork. Optional measured fitting sizes and aligns their painted bounds
  within tiles and sheet stages, retaining each visible instance's multiplier.
  Focused rendering and cache regressions cover visible previews;
  populated-world and displayed-frame acceptance remain pending.
  Personal panels and HUD editor surfaces can bind the host local player's name
  and skin head, with neutral artwork when the identity is unavailable. Skin and
  session changes retire the previous portrait; full in-world acceptance remains pending.
  Embedded panel fonts admit covered Latin-1 and General Punctuation glyphs within
  the existing atlas bounds. Active native fields retain an accent focus border;
  focused regressions cover glyph fallback and steady-frame geometry reuse.
  populated-world and displayed-frame acceptance remain pending. Panel motion
  preferences now settle feedback, wheel scrolling and surface transitions
  immediately, retain prepared routes and apply across route changes.
  displayed-frame pacing and complete rendering acceptance.
  Saturation overlays prepare source outlines on image workers independently of GPU-sheet
  capacity and area-filter them to the requested draw size. Paint skips pending sources,
  unions small masks within the visible viewport and atlas allocation budget, and draws
  ordinary prepared sprites. A small LRU reuses clamped source grids across GUI scales. Current and held-food
  coverage share one mask. CPU/GPU regressions cover tint, cropping, unequal translucent
  edges, independent shake, inversion and high-resolution minification; complete in-game
  and platform acceptance remain pending.
  HUD layout editing stores nine anchors and GUI-pixel offsets after completed
  moves, resizes and nudges, with an additive anchored-layout read and normalized
  compatibility reads. Editor drags snap to neighboring cards, stacking gaps and
  host-supplied native HUD bounds, with edge distances and Shift snap bypass.
  The editor also retains bounded undo/redo history, per-card draft visibility,
  anchor and scale actions, live owner-scoped card updates and inspector bindings.
  Grid display and snapping have independent controls. Published cards can start
  hidden, and optional inspector dimensions place settings beside the selected
  card with viewport clamping and retained geometry bindings.
  Hidden card IDs have an additive result read; toast and anchor motion use the
  presentation clock. Displayed-frame pacing and populated-world Studio acceptance
  remain pending.
  Backdrop and element blur now limit downsampling to referenced mip levels and
  cache the admitted maximum blur. Pixel-equivalence regressions are included;
  executed GPU checks and measured frame-budget acceptance remain pending.
  Server-day HUD rows now consume scoped native clock observations and leave missing
  or stale clocks unknown. Clock/host regressions are included; executed checks and
  populated-world acceptance remain pending.
  acceptance remain unverified in a displayed client. Packet-extension world
  drawing uses the retained geometry/depth owners, actor-mask compositor and
  full-block highlight renderer. Headless camera, box/tracer and menu captures
  exist; actor silhouettes, coloured full-block highlights and broad platform /
  hardware-performance acceptance still require validation.
- Enhanced rendering remains restricted until its GPU faults and freezes are resolved.
  Diagnostics or bounded fixtures do not establish safe general gameplay use.
  Its MSAA nametag composite blends against resolved scene colour, so labels partly
  occluded by world silhouettes get a dark edge fringe; per-sample composition is incomplete.
- Java shader packs (Iris/OptiFine format) run behind the off-by-default `shaderpacks`
  feature. `shaderpack` loads packs, parses `shaders.properties`, options, Iris option
  menus, profiles and option files, evaluates custom uniforms and translates programs to
  validated naga modules, with calls ordered after forward declarations and generated
  helpers. The render host runs Iris's frame (shadow, gbuffers for terrain, actors, block
  entities and the hand, deferred, composite and final passes, compute, custom textures and
  images) plus the pack-drawn sky, particles, clouds, weather, lightning, cracks and selection.
  Block, item, entity and biome ids and Java sky and fog colours come from the pinned
  Bedrockâ†’Java mappings. Settings offers pack import, a reload key, load status and a Shader
  Pack Settings screen. Incomplete: in-game acceptance across packs and platforms (Metal is
  unverified); stage pre-flips (`flip.<stage>_pre.<buffer>`) and Iris's explicit-flip
  ordering; drawing the solid hand's glint before deferred passes; pack replacements
  for uncovered celestial layers (the pack sky stands), actor colour masks, multitexture
  materials, dissolve pairs, beacon beams and additive rays (these retain native
  composition over the pack); native layers behind pack-rendered glass or water, whose
  final depth can hide those layers; autofocus on cutout
  Bedrockâ†’Java mappings. The Shaders settings tab offers pack import, reload, load status
  and a Shader Pack Settings screen. Video links to Shaders and shows its renderer switch
  only when Enhanced is available.
  Incomplete: in-game acceptance across packs and platforms (Metal is unverified); stage pre-flips (`flip.<stage>_pre.<buffer>`) and Iris's explicit-flip
  ordering; drawing the solid hand's glint before deferred passes; geometry-host composition
  of celestial layers, effects without depth writes and private-depth first-person scenes,
  which keep the native scene copy; actor colour masks, multitexture materials, dissolve
  pairs, beacon beams and additive rays, which keep native composition; autofocus on cutout
  pixels, liquids and entities; complete Iris type checking and target-dependent return
  overloads; constant folding for half-float packing and bit casts. Java sky and fog time
  curves and sunrise colours remain provisional.
- `registryc --kind java-mapping` generates `crates/assets/data/java-mapping-v2193.bin` from
  the GeyserMC data pinned in `assets/geyser-mappings-source.json`: Java block states, block
  tags, biomes and their colours, entity types and items for every Bedrock id Geyser maps.
  States Geyser never produces take the closest same-block state and are flagged approximate;
  Bedrock-only biomes and blocks stay unmapped. Java shader packs read it for their ids,
  biome uniforms and sky colours.
- Runtime pack compatibility still includes custom texture inheritance/tints, authored
  geometry and block/item component behavior beyond the currently supported cases.

See [Android](docs/android.md), [mods](docs/modding-spike.md),
[server experiences](docs/server-experiences.md) and
[experience runtime](docs/experience-runtime.md) for existing implementation contracts.

## Acceptance and release qualification

Use the current [live-testing guide](docs/agents/live-testing.md) as the single owner of
reference hardware, frame/hitch, streaming, join and capture budgets. Capture the
rendered result on the relevant platform and scale; distinguish hidden-window pixel
coverage from displayed-frame pacing and OS input/focus evidence.

Compiled entities, blocks and icons share a compressed disk cache. Hits restore dependency
records; archive, session, startup artwork and device inputs invalidate affected entries.
Actor artwork prepares current demand immediately and prewarms unused pages within a
per-frame budget. Artwork arrays allocate exactly their source layer count. Skin slots
keep geometric capacity. Each artwork slot retains its own unchanged texture and bindings.
Worker rig-plan preparation and measured cold arrival latency remain open acceptance work.
Hardware join and frame acceptance still needs the full capture matrix below.

Check deterministic work in CI: bounded queues, invalidation, stale-work rejection,
retained memory, allocations, uploads and unnecessary rebuilds. Measure release
performance on hardware; existing source refactors and unmeasured optimizations
remain incomplete performance evidence. Preserve packed terrain storage and shared
render resources while measuring implementation choices under the architecture policy.

Release qualification starts from a clean install and covers sign-in, a server,
a Realm and a friend's world, survival movement/building/mining/containers/crafting,
chat/forms, resource packs, and an offline local world followed by reload. Required
platform, parity and performance evidence must be complete for the claimed release scope.
Keep the completion ledger consistent with this checklist; no historical green test
or isolated fix silently completes the surrounding gate.

## Native Recording Studio (in progress)

Scope: an integrated main-screen studio for recording and editing gameplay,
PvP highlights and cinematics. The editor needs replay camera modes and custom
follows, an editable multi-track timeline, event-driven automatic effects, media
and audio import, titles, transitions, project persistence and finished video
export. Agent/MCP editing is a later stage and does not replace these native flows.

Current implementation; runtime acceptance remains incomplete:

- A Bevy-free project owner admits bounded metadata and cross-references. Rational
  frame rates and integer media time drive preview and export plans. Transactional
  edit commands, bounded undo/redo, camera rigs, effect curves, audio fades, titles,
  markers and transitions retain immutable source media.
- Combat analysis retains evidence confidence. Correlation between an attack
  attempt and a server hurt/death observation is labelled estimated, rather than
  reported as confirmed attacker attribution. Automatic effects remain ordinary
  editable commands; draft limits report omitted events.
- Recording storage uses a bounded worker queue, streaming compression, multipart
  checkpoints, append-only segment commits, integrity checks, a disk quota and
  observable completion/errors. Seek readers restore a staging scene from the
  nearest committed checkpoint and support cancellation. This is the container;
  native prepared-scene adapters and a shared periodic checkpoint policy are now
  present in source. Full received-world coverage and native playback rendering
  remain incomplete.
- Asynchronous project storage uses exclusive staging and atomic publication,
  revision-tagged completions, portable media references and bounded library
  enumeration. The main-screen Recording Studio entry and native library/editor
  connect these services to project create/open/save, undo/redo, split/delete,
  speed changes, titles, frame stepping and pointer scrubbing. The preview viewport
  now connects raster media and a provisional native replay source in code;
  this UI remains unverified.
- Native capture controls now arm recording from the launcher and start/stop or
  bookmark from pause/studio. Startup and finalization use workers; the live path
  bounds GPU readbacks, queued video bytes and mixed-audio storage independently.
  A passive audio tap preserves speaker playback. NVIDIA encoding is probed before
  the capture clock starts, with an Auto software fallback; long WAV tracks use
  RF64 and live MP4 output uses recoverable fragments. These paths are unrun.
- The live POV-video schema retains actual camera metadata and admitted
  attack/server hurt/death observations. Completed takes open as video, linked game
  audio and reversible automatic highlight markers/effects, with estimates and
  omitted observations reported. Linked audio follows visual edits; generated
  effects do not yet follow subsequent clip retiming.
- A separate native terrain track now captures a prepared-mesh baseline, changed
  publications and unloads alongside POV media. Its worker freezes exact runtime
  material/model/texture tables, per-layer UV metadata, mip chains and biome tints
  into hash-addressed local objects. Periodic checkpoints reuse object references;
  seek restoration validates a staging scene with explicit memory limits. Live
  admission shares immutable geometry, bounds pinned bytes, and stops on worker
  failure instead of silently dropping world changes. Cancellation reaches large
  asset reads; capture manifests retain per-track completion and failure details.
  Terrain schema v2 explicitly selects the active texture/tint palette, even in
  empty scenes or geometry-free pack changes; checkpoints retain the selection
  and share its resident allocation charges with chunk references. Experimental
  v1 terrain tracks lack that state and remain unsupported for native playback.
  This source is unverified.
  Terrain alone does not establish complete 3D replay; the provisional viewport
  below still lacks complete actor coverage and full preview composition.
- A prepared environment track now records atmosphere channels, classic lightmap
  inputs, fullbright state and the texture animation clock at the take's frame
  rate. Immutable sky textures, fog/biome profiles and optional dimension skyboxes
  freeze independently of the install with bounded pinning and object validation.
  Reference-only state checkpoints bound seek history. Native startup, sampling,
  stop/error reporting and per-track manifest completion are connected in source.
  This does not implement biome-aware lighting for a moved replay camera, live
  replay composition or shader-mode restoration; these paths remain unverified.
- A prepared actor track now retains GPU instances, previous/current bone poses,
  exact draw identities, geometry spans, skin residency and artwork/glint rasters.
  Native start/sample/stop and manifest status are connected in source. Immutable
  epochs share bounded leases and hash-addressed objects; dynamic poses remain
  journal records. Restore validates geometry, pose and raster cross-references.
  This provisional source covers the original camera's culled draws and lacks its
  hidden first-person body. Unculled received-world coverage, pose interpolation
  and renderer publication remain incomplete; this does not close a replay gate.
- Incremental source cursors now retain verified segments and advance only newly
  crossed records. Large checkpoints spill into bounded delete-on-close scratch
  storage instead of growing the playback heap. Whole-segment checksum/framing
  admission precedes callbacks; failed/cancelled operations require a fresh seek.
  Terrain playback emits changed bindings and avoids rebuilding redundant later
  checkpoints. Actor seeks decode only the last crossed pose; immutable asset
  caches remain shared. A coordinator admits terrain, environment, actor and
  original-view camera tracks together, rejects mixed take metadata and limits
  interrupted playback to their common duration. Camera capture now checkpoints
  its actual pose at the shared scene interval. The provisional native viewport
  below consumes these unverified paths; synchronized audio playback is missing.
- Raster media preview now connects saved timeline sampling to the native editor's
  image surface. A bounded worker owns local FFmpeg decoding, pixel composition
  and hashing; forward requests coalesce without cancelling every frame, while
  new seeks/documents cancel obsolete work. Native UI pages reuse a dedicated
  bounded slot outside server/mod texture ranges. Crops, opacity, ordered tracks,
  black gaps and grouped crossfades have source implementations; ordinary opaque
  single-source frames share decoded storage. This provisional raster path uses
  point crop sampling and lacks source-identity/frame-accuracy acceptance. Titles,
  camera/scene effects and native replay scenes use the app adapters below;
  synchronized audio preview remains missing. No automated tests or rendered
  acceptance were performed; this does not close the Studio gate.
- A provisional native replay viewport now consumes the coordinated scene frame
  in an independent headless app on the main device. Prepared terrain, actors,
  environment and animation clock stay separate from live authorities. The native
  inspector can switch a captured POV clip to a replay reference through undoable
  metadata commands. Source changes rebuild the context; seeks replace camera
  view history. Native allocation/pipeline readiness precedes image publication.
  A bounded four-target pool, GPU-only UI copies, retained-layer damage and queue
  completion leases avoid frame readbacks and target overwrite. Shared device
  faults retain the primary app's fatal-error path when the worker is idle/closed.
  This remains unverified at runtime. It supports one unframed replay layer and
  the camera authoring described below; unculled
  actor coverage, HUD/hand/particles/block entities, exact projection planes,
  shader modes, interpolation, audio and complete visual composition
  remain incomplete. This does not establish replay parity or measured efficiency.
- A separate bounded camera-target journal now captures received actor anchors,
  names, angles and spawn identities, with the local subject's published eye offset.
  Changed-only records, recycled capture buffers, worker serialization and periodic
  checkpoints avoid full idle snapshots. Verified coordinated playback accepts
  this optional track only from the same take; old takes remain readable without it.
  This does not expand the original-view actor draw coverage.
- Native camera controls now author player/free/path/follow/orbit shots, target
  browsing with explicit retargeting, fine parameter adjustments, exact-playhead
  path keys and boundary trims through ordinary undoable project edits. New actor
  shots pin spawn identity; unavailable subjects disclose their recorded-view
  fallback. Remote first-person eyes are not inferred. Both preview paths sample
  the output frame grid, while camera metadata edits reuse admitted scene state.
  Target interpolation, smooth camera motion, full actor coverage, visible key and
  easing controls, narrow-layout inspector access, and runtime/render acceptance
  remain incomplete. Camera behavior has not been exercised at runtime.
- Native Export now snapshots the current edit, evaluates each output frame on the
  shared rational grid and supervises exact-order hardware/software encoding on a
  worker. It connects raster composition and one isolated native replay layer,
  reuses bounded GPU readback/CPU storage, and mixes audio in bounded blocks through
  disk-backed stereo float scratch before AAC muxing. Trims, rate, gain, mute,
  fades and silence derive from saved audio metadata. Progress/cancellation persist
  across library/editor views. Successful encoder/mux completion and file sync
  precede no-clobber publication; temporary export material is ignored.
  This source has not had runtime acceptance. Preflight rejects
  native layering/crops/crossfades and camera shots outside replay
  coverage; missing subject payloads can still fail at a later frame. Full export
  composition, settings/destination controls, audio preview, source accuracy,
  interrupted recovery, playback/export acceptance and measured cost remain open.
- Shared GPU frame effects now connect raster/native preview and export for
  exposure, saturation, vignette, bounded separable bloom, flash, letterbox and
  fades/dip-to-black. Replay camera effects stay in the world camera; ordinary
  video uses provisional affine image motion. Neutral frames skip composition;
  targets/scratch/upload storage are reused, backpressure precedes pixel uploads,
  and native effect preview retains GPU-only image publication. Shader uniforms
  generate their WGSL fields from the Rust definition. No shader/runtime validation
  or measured performance acceptance has been performed.
  Paused color edits reuse the admitted base image when source time/camera match;
  seeks bypass that cache and failed scene application/drawing requires fresh
  source admission.
- Native effect authoring now adds/enables/disables effects, edits exact amount
  keys and incoming easing, navigates/deletes keys and shifts effects through
  ordinary undoable commands, preserving gameplay origin. A bounded curve graph
  and decimated key markers expose authored data. Cached interval packing separates
  overlapping camera/effect/audio/title items without changing render precedence;
  visible-row culling and source-name lookup avoid repeated offscreen text work.
  Title runtime acceptance, synchronized audio preview, automatic-effect retiming,
  full native visual composition, numeric/drag controls, narrow-layout interaction,
  capture/seek/export acceptance and measured resource/frame costs remain open.

- Editorial title overlays now share worker-owned layout, installed font snapshots,
  bounded immutable ink caches and GPU composition between preview and export.
  Titles remain screen anchored after scene camera/color effects, with independent
  opacity envelopes. Native multiline text drafts apply as one undoable edit;
  font role, color, placement, size, shadow, fades, frame moves and trims use saved
  metadata. Explicit-line caret navigation and pointer placement share the native
  text editor. This is provisional source: font rendering, composition, editing,
  cancellation and resource budgets still require runtime acceptance.

- Windows media finalization now opens completed encoder output with write access
  before flushing, avoiding read-only handle failures during recording and export.
  FFmpeg discovery also reads updated Windows installation paths without changing
  the process environment, with portable and explicit executable-path options.
  Native terrain-track failure retains POV video capture and reports incomplete
  3D replay coverage; it does not establish successful terrain capture. Capture and
  storage failures now retain diagnostics, and unfinished takes explain their
  missing manifest. Runtime acceptance of these recovery paths remains open.

- Frozen runtime animation spans now repack shared or reordered frame ranges
  without changing animation indices, texture order or timing. Carrier validation
  remains strict. Live video capture uses three ordered readback slots within a
  byte budget, transfers the final screenshot consumer's allocation directly and
  publishes progress on its quantized clock. Immutable actor pixels reuse bounded
  weak publication leases. Recording/export muxing retains encoded video packets
  instead of trimming them to audio packet timing. Playback and performance
  acceptance for these changes remain open.

- Interactive video preview now uses bounded hardware-decoded read-ahead on
  Windows, with a software fallback and a bilinear preview scaler. Export keeps
  its existing quality path. The transport advances only within two admitted
  output frames, discarding catch-up debt while decoding or rendering is late.
  Video and native previews share retained GPU publication; per-frame raster
  hashing no longer rebuilds the UI texture catalog when a GPU is available.
  Preview source, decoder selection and preparation latency are logged separately
  from capture misses. Existing-media decoding was inspected, but in-app playback
  smoothness, fallback behavior and long-session budgets remain unverified.

- Native replay coverage now publishes received actors before camera culling,
  interpolates actor/target poses across continuous samples, restores dimension and
  renderer settings, and records optional hand/HUD plus prepared particles,
  weather, overlays and independent received-column block models. Explicit legacy,
  warmup, capacity and unreceived-world limits remain visible; new tracks cannot
  restore inputs omitted from older takes. Runtime visual parity remains open.

- Cinematic paths now support time-aware smoothing, shortest-angle rotation,
  editable keys/easing and typed camera values. Shared source-time speed ramps
  drive visual sampling, linked game audio, transitions and source-anchored
  generated effects/markers. Ramp audio now uses shared pitch-preserving tempo
  processing; perceptual qualification remains open. Native/raster
  layers share a linear premultiplied preview/export compositor with crop,
  opacity and crossfades. These implementations still need rendered acceptance.

- The editor now has horizontal wheel panning, pointer-centered zoom, a project
  overview, track scrollbars, drag moves/trims, optional playhead following and
  compact Preview/Media/Inspector panels. Repeated unchanged pointer seeks no
  longer invalidate preview preparation. Looks and additional color/detail
  effects remain editable curves; generated hit edits use bounded, varied recipes.
  Import/relink, bounded source thumbnails, capture/output settings and destination
  editing run through the existing worker and undo owners. Interrupted POV video
  can be remuxed to a separate recovered file; incomplete native journals are not
  reported recovered. Interaction and long-session acceptance remain open.

- Requested capture/output cadence includes 120/240 FPS. Capture preparation is
  admitted at the selected cadence rather than every uncapped client frame. New
  capture sizing preserves native scene coordinates and scales final GPU output
  before readback; sustained cadence still depends on render/encode throughput,
  with missed slots reported explicitly. No measured high-FPS claim is made.

- Actor recording now reuses unchanged raster snapshots even when the live
  renderer replaces their outer artwork wrapper. Queued geometry and appearance
  epochs charge shared immutable allocations once, retain the existing total
  cap, and report retained/requested/limit bytes on real exhaustion. This addresses
  immediate actor-budget interruptions; this follow-up is formatted only, with
  compilation and live recording validation pending.

- External recording cameras now request a separate local world-body sample.
  Native controller selection, bone visibility, skin layers and motion endpoints
  use the world perspective without changing the live first-person hand state.
  Previously hidden body poses cannot be recovered from existing takes. This
  follow-up is formatted only; compilation, regression and rendered acceptance
  for native/Java animation, skins and external cameras remain incomplete.

- Studio playback now retains ordered GPU readbacks during encoder backpressure,
  records native camera samples independently of video admission, reuses the local
  world-body frame, and updates crossfade budgets without rebuilding replay contexts.
  Interactive view drafts coalesce without continually cancelling in-flight renders.
  Transient shader loading remains pending rather than becoming a permanent failure.
- Device-paced Studio audio now shares bounded decode/mix and pitch-preserving
  retiming with export. Epoch cancellation, callback-confirmed pause reconciliation,
  unique worker jobs, mute/gain/fades and video ceilings coordinate its clocks.
  Preview currently admits at most 32 simultaneous audio streams; hardware sync and
  perceptual time-stretch qualification remain incomplete.
- The native camera viewport now supports focused WASD/QE flight, right-mouse look,
  recorded-subject markers, projected paths, draggable keys and evaluated Bezier
  handles. Inspection views remain transient until applied. Spatial authoring is
  limited to one native layer with default framing; subjects and path keys use
  visible bounded windows. Legacy captures cannot recover omitted local-body poses.
- Timeline selection, linked duplication/group moves, magnetic snapping, track
  mute/lock and direct key/curve drags use ordinary admitted edit transactions.
  Track mute persists; editor locks and panel preferences remain local view state.
  The existing transaction-size limit remains explicit.
- Studio panels now separate creation tools from selected properties, retain the
  viewer/transport/timeline, and expose density, panel visibility and timeline sizes.
  Anonymous production-painter snapshots render at wide, short and narrow sizes,
  including audio and settings; the former short-window inverted rectangle is fixed.
  Focused changed-crate checks compile. Regression and runtime acceptance is being
  exercised separately; no recording-FPS gain or complete replay parity is claimed.

- Production hardening now isolates recording actor preparation from live artwork,
  reuses same-clock animation samples and bounded GPU readback buffers, avoids
  recompressing scene envelopes, and limits live software encoder threads. Capture
  health reports CPU admission/actor time, readback/encoder queues and stalls.
  The reported 160-to-50 FPS recording regression has not been remeasured; no
  minimal-overhead or sustained-capture gate is closed.
- Preview has independent Fast/Balanced/Full resolution and playback budgets,
  exact paused frame stepping, bounded decoded-frame reuse and adjacent-cut
  context reuse. Audio credit follows preview cadence rather than export FPS.
  Export retains exact output settings, checks used sources, validates MP4 framing
  and publishes without overwriting an existing destination. These mechanisms
  still need long playback, seek, slow-disk, cancellation and hardware qualification.
- Project recovery uses caller-clock autosaves, bounded versioned copies, ordered
  checkpoints, durable discard revisions and explicit restore into an unsaved copy.
  Setup checks actual encoder operation; tabs and a shared keyboard/help registry
  simplify workspace controls. Recovery and dependency screens have anonymous
  snapshot fixtures. New owning regressions are written but unrun. This hardening
  pass has no builds, tests, rendered captures or runtime acceptance.

- Replay-reference controls include stationary entity tracking, independent shot
  FOV curves, limited new-player POV anchors and lifetime-bound body hiding.
  Follow/Orbit creation retains inspected framing. Remote POV admits only upright,
  unit-scale, unmounted living standing players; other poses, mobs, missing legacy
  anchors and transition eye easing remain incomplete.
- Monitoring exposes runtime-only 0.1xâ€“8x speed, typed seeking, seconds jumps and
  whole-project looping with rational audio cursor conversion and bounded tempo
  processing. Fast preview respects its wall-time presentation budget. Slow
  monitoring retains the existing frame grid; gapless looping remains incomplete.
- Saved export In/Out uses distinct visual-frame and PCM admission. Library
  search/sort runs on the storage worker before visible truncation; painted row
  generations and captured paths guard refreshes and dirty-open confirmation.
  The reference's unspecified Module item uses existing keyframed Effects.
  New regression sources are unrun. These additions have source review and
  formatting only; compilation, rendered acceptance, synchronization and export
  qualification remain incomplete. No parity or performance gate is closed.

Required work remains open:

- Preview refreshes now retain admitted same-epoch camera images and font owners,
  with bounded one-frame preparation ahead of playback. The viewer defaults to
  uncluttered navigation; player and path markers require explicit tools. Left
  drags orbit transiently, while saved image rotation and Fit/Fill use one CPU/GPU
  transform. Focus view and native content fitting enlarge the usable picture.
- Player selection uses versioned captured actor classification. Native HUD/chat
  is hidden by default independently of project titles and prepared world tags.
  New visual captures retain exact nametag quads and content-addressed atlas cells;
  older takes cannot recover missing classifications or tag artwork. New clean
  POV capture omits screen overlays without hiding the live HUD. These follow-up
  changes are source-reviewed and formatted only: their owning regressions,
  compilation, capture ordering, seek/audio smoothness and rendered acceptance
  remain incomplete. No performance or visual parity gate is closed.

- Render and exercise recording options, library/editor, focus, scrolling,
  numeric/drag timeline edits, loading/error/recovery states and dirty-edit
  confirmation on wide, narrow and short windows.
- Validate received terrain/actors/appearance, native visuals, lighting, time,
  camera and gameplay evidence across session/dimension boundaries. Independent
  playback must preserve identities and explain omitted, stale or limited data.
- Exercise continuous playback, seeks, cancellation and temporal resets with
  player/free/path/follow/orbit cameras and mixed native/raster compositions.
  New visual tracks cannot reconstruct data omitted from older takes.
- Validate linked edits, imports/relinks, speed ramps, generated event anchors,
  thumbnails/waveforms, audio export and interactive audio preview/mixing.
  Hardware synchronization and additional assisted editing workflows remain open.
- Validate hardware/software export, progress/cancellation, destination safety,
  completed-file publication and interrupted POV recovery. Native journal repair,
  long-session disk quotas, resize and shutdown acceptance remain open.
- Owning-crate regressions and rendered/runtime acceptance are still required.
  Performance, sustained capture cadence and resource bounds need measurement.
  Earlier Default and Enhanced production checks predate the latest hardening.
  The latest hardening is source-reviewed and formatted only; compilation, new
  regressions, rendered acceptance and game launches remain unrun.

This feature is incomplete. Domain APIs and storage code alone do not establish a
working Recording Studio, replay visual parity or a performance acceptance result.
