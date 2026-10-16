---
name: Cinnabar Launcher
description: A floating cinnabar outcrop at dusk, with one lit block of a Play button.
colors:
  cinnabar: "#e2483a"
  cinnabar-hi: "#f27a63"
  cinnabar-fill: "#d23b2e"
  cinnabar-ink: "#fff7f3"
  ink: "#f5f0ea"
  ink-2: "#c9c2bb"
  ink-3: "#aba59f"
  solid: "#11131d"
  glass: "rgb(14 16 28 / 0.66)"
  glass-strong: "rgb(12 14 24 / 0.8)"
  glass-edge: "rgb(255 255 255 / 0.07)"
  raise: "rgb(255 255 255 / 0.06)"
  raise-2: "rgb(255 255 255 / 0.1)"
  line: "rgb(255 255 255 / 0.08)"
  ok: "#6fd39f"
  warn: "#f2b550"
  bad: "#ff7a8a"
  focus: "#ffd0c4"
typography:
  hero:
    fontFamily: "Archivo Variable"
    fontSize: "64px"
    fontWeight: 800
    lineHeight: 0.92
    letterSpacing: "-0.025em"
    fontVariation: "'wdth' 125"
  page:
    fontFamily: "Archivo Variable"
    fontSize: "34px"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "-0.02em"
    fontVariation: "'wdth' 125"
  play:
    fontFamily: "Archivo Variable"
    fontSize: "23px"
    fontWeight: 800
    lineHeight: 1.05
    fontVariation: "'wdth' 125"
  title:
    fontFamily: "Archivo Variable"
    fontSize: "19px"
    fontWeight: 700
    lineHeight: 1.2
  body:
    fontFamily: "Archivo Variable"
    fontSize: "15px"
    fontWeight: 450
    lineHeight: 1.5
    fontFeature: "'tnum' 1"
  label:
    fontFamily: "Archivo Variable"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: 1.3
  meta:
    fontFamily: "Archivo Variable"
    fontSize: "12px"
    fontWeight: 500
    lineHeight: 1.35
rounded:
  sm: "8px"
  md: "12px"
  lg: "18px"
  pill: "999px"
spacing:
  s2: "8px"
  s3: "12px"
  s4: "16px"
  s5: "24px"
  s6: "32px"
components:
  play-ready:
    backgroundColor: "{colors.cinnabar-fill}"
    textColor: "{colors.cinnabar-ink}"
    typography: "{typography.play}"
    rounded: "16px"
    height: "76px"
    width: "360px"
  play-busy:
    backgroundColor: "{colors.glass-strong}"
    textColor: "{colors.cinnabar-ink}"
    rounded: "16px"
    height: "76px"
  join:
    backgroundColor: "{colors.raise-2}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    height: "34px"
  join-hover:
    backgroundColor: "{colors.cinnabar-fill}"
    textColor: "{colors.cinnabar-ink}"
  shelf:
    backgroundColor: "{colors.glass}"
    rounded: "{rounded.lg}"
  settings-group:
    backgroundColor: "{colors.glass-strong}"
    rounded: "{rounded.lg}"
  segmented-thumb:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.solid}"
    rounded: "{rounded.pill}"
---

# Design System: Cinnabar Launcher

## Overview

The window is a world, not a page. A real-time voxel outcrop of slate and dolomite, split by a glowing cinnabar vein, floats in a sky lit for the player's local time of day. The player stands on a small stone in the foreground, near enough to the camera to read as the scene's figure, and turns their head toward the pointer. The interface floats over it: the title, one lit Play block, and a single glass shelf. Everything except the scene is quiet, exact and fast. Spectacle is spent once.

The vein is the signature move. It is the product state made visible: it refines from dark ore to glowing cinnabar as an update downloads, and goes dark when the installed build cannot start.

Anti-references: ad rails and promo carousels (Lunar, Dawn), dense rails of unbuilt features, grey dashboards with one accent, flat isometric clip art.

## Colors

- **Cinnabar** (`#e2483a`, fill `#d23b2e`, highlight `#f27a63`) comes from the brand mark's faces. It marks the one primary action, the vein, and the current location. It never decorates.
- **Ink** is warm dolomite white in three steps. `ink-3` is the floor for text on glass and holds 4.6:1 over the brightest part of the dusk sky.
- **Glass** is smoked and tinted toward the night sky, never neutral grey. It appears only where content sits over the world: the shelf, notices, the account chip and settings groups.
- State colours are reserved for state: `ok` for live and good ping, `warn` for recoverable trouble and stale data, `bad` for a build that cannot start. Each always pairs with an icon or words.
- Scene palettes (sky, light and ore glow per time of day) live in `src/lib/diorama/mood.ts`, not in CSS.

## Typography

One family, Archivo (OFL), carries everything. Display roles use its expanded width (`wdth` 125) at weight 800. UI roles stay at normal width. Numerals are tabular throughout. Sentence case everywhere: no all-caps labels, no eyebrows.

## Layout

- The title bar is 56px and transparent: wordmark, three text tabs with a sliding cinnabar bar, and account at the right. On macOS the left 88px belong to the traffic lights.
- Home is a two-row grid. The launch column floats left and vertically centred, the island is framed right of centre (`FRAME` in `App.svelte`), and one glass shelf (news, then three server cards) runs along the foot.
- Secondary pages veil the world: it blurs, dims and stops rendering, and the content column sits over it. Settings caps at 720px, Servers at 1240px.
- When the window is shorter than 720px or narrower than 1100px, the title shrinks and the shelf shows only the lead headline.

## Elevation & Depth

Depth comes from the scene: an atmospheric sky, fog, soft shadows, baked voxel ambient occlusion, a cool rim light, bloom on the ore, drifting rock chunks that parallax with the camera, and a left scrim that shapes light behind the text. Interface surfaces use glass with a 1px inner edge and no outer shadow. The only cast shadow belongs to the Play block, a soft cinnabar glow offset below it.

## Shapes

Radii step by hierarchy: 8px for small controls, 12px for rows and tiles, 16px for Play, 18px for cards and groups. Pills are reserved for chips, the join button, search, filters and segmented controls.

## Components

- **Play** is the state machine. Ready is a lit block face: a top-down gradient, an inner top highlight and a hover lift with one band of light. Busy is glass with a cinnabar fill and a light sweep confined to the filled part. Running shows a live dot. Labels cross-fade and rise between states.
- **Server cards** show typographic poster art (a hue field, a fixed-scale block lattice and the initial) whenever a server ships no image. Hover or focus reveals the MOTD, tags and Play.
- **Segmented control**: a dark track with an ink thumb that slides, operated with arrow keys.
- **Notice**: glass with a tone-coloured inner edge and icon. Its action is the recovery step. The `info` tone carries deliberate choices, such as a downgrade.
- **Update outcomes**: a downgrade never takes over Play. It is a quiet notice and a Settings row that open the platform installer. Installing names its outcome: on Windows, "Opening installer" (the launcher closes); elsewhere, "Installing" (the launcher restarts into the game).
- **First run**: until the game has signed in once, the server list explains why it is empty and offers Play.

## Do's and Don'ts

- Do spend colour on Play, the vein and the current location only.
- Do keep the world rendering only on Home and while the game is not running. Reduced motion and slow GPUs get a still frame. Without WebGL, the engine's own pre-rendered still (`src/assets/world-still.jpg`) stands in.
- Don't add destinations, chips or badges for features that don't exist.
- Don't use middle-dot meta strings, all-caps labels or eyebrows.
- Don't use game assets, names or trademarks. Every texture, skin and poster is generated in `src/lib/diorama/` and `src/lib/poster.ts`.
