# Native first-person offhand placement

This records identified native behavior; it does not claim that every route below is
implemented or visually accepted.

## Separate render path


Maps, modern attachables, blocks, authored display transforms and legacy items have
distinct routes. The default legacy route is admitted by the item fallback or the
non-old-tessellation block display flag. Outside that route, the source selects the
authored world-matrix transformation instead.

## Default legacy sprite matrices

The following are post-multiplied onto the incoming camera matrix, in written order.
Angles below are degrees. These are native tessellator-frame matrices, not an instruction
to apply them directly to an unrelated centered mesh basis.

For `Item::isHandEquipped() == false`:

```text
Rx(90)
* T(-1.25, -1.125, 0)
* Rz(-80)
* Ry(-20)
* T(-0.3125, 0.25, -0.03125)
* S(1/16)
* S(16/max(icon_width, icon_height))
```

For `Item::isHandEquipped() == true`, unless the legacy Shield-blocking special case wins:

```text
T(-0.6875, -0.125, -1.53125)
* Ry(-10)
* Rx(70)
* Rz(80)
* S(1/16)
```


Transform constants:

| Value | Use |
| --- | --- |
| `pi/2` | Flat sprite X rotation |
| `1.25` | Flat sprite negative X translation |
| `-1.125` | Flat sprite Y translation |
| `-80`, `-20` degrees | Flat sprite Z/Y rotations |
| `0.3125`, `0.25`, `0.03125` | Final flat sprite translation |
| `0.6875`, `-0.125`, `1.53125` | Hand-equipped translation |
| `-10`, `70`, `80` degrees | Hand-equipped rotations |
| `0.0625` | Native pixel-to-model scale |

The table expresses rotations in degrees. Offhand depth is distinct from
the ordinary block depth `0.72`.

## Geometry normalization


## Blocks and Shield legacy alternative


The legacy Shield-blocking route uses X/Z/Y rotations `2.5`, `177.5`, `-2` degrees,
then its offhand-height-dependent translation and scale `1.125`.
Modern Shield attachables instead use their authored animation and owner binding; see
[held attachables](held-attachables.md) and [blocking state](shield-blocking.md).

## Offhand render context


Incomplete gates: the complete map/legacy Shield route, authored display transforms,
and stack-specific native instant-update/equivalence predicates (the current retained
equipment feed supplies identifiers, not the full native cached ItemStack comparison).
The October 1 offline vanilla-BDS run on macOS/Metal at Retina scale 2 renders
the offhand Shield alongside main-hand blocks and the Crossbow, with independent
texture bindings and poses. The open survival inventory shows its offhand icon;
real take/place/restore gestures leave both that icon and the hand visible.
Geometry, clipping, layering, scale, colors and input ownership were inspected
in fresh rendered frames. This is functional/device rendering evidence, not a
matched native gallery or acceptance of the incomplete routes above.
