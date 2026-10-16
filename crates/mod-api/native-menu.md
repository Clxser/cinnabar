# Trusted packet-mod menus

A WASI preview 1 packet reactor may export `menu(i32, i32) -> i32` alongside
`native-init(i32)`. The host sets capability bit `2` only when the mod has the
controls grant. Camera frames use bit `1` independently. Reactors without either
optional callback retain their existing packet ABI.

`menu` receives UTF-8 JSON bytes. Its return uses the same canonical
`result<list<u8>, string>` layout as `frame`: a four-byte discriminant, then the
pointer and length of the successful byte list or error string. Exporting
`cabi_post_menu(i32)` releases the returned allocation after copying it.

Input is bounded to 64 KiB:

```json
{"version":1,"open":true,"viewport":[640,400],"events":[{"id":"enabled","value":1}],"keys":[{"key":"KeyA","text":"a"}]}
```

The optional `viewport` is the available authored GUI size before reference-size fitting; it follows window resizing and platform safe-area insets.

The host delivers native control IDs and values. Focused menu keys may carry
composed text; native number and choice editors receive their keyboard input
first. `capture_key` lets the guest handle Escape and the panel's toggle key while
open. The toggle key reopens the retained menu while closed. Other screens and
unfocused windows receive no guest menu keys.

Output is bounded to 512 KiB. JSON `null` retains the current menu. A changed
output contains `open`, `panel` using `ui::mod_panel::Panel`, and optional `keys`:

```json
{"open":true,"panel":{"title":"Example","toggle_key":"ShiftRight","dark":true,"controls":[]},"keys":["KeyR"]}
```

`keys` subscribes to physical key press edges while focused gameplay owns input.
Those keys are reserved from ordinary gameplay, never collected from chat or
another menu. Each native packet session has its own menu identity, input queue
and prepared catalog; reconnecting or revoking grants retires its pending input
and retained presentation. Calls run on the existing serial packet worker.

Custom panels support surface bindings, eased transitions, glass and paint
transforms through the shared JSON-UI engine. The `mod_panel_sans` font alias selects the shipped Inter outline face. `mod.control:N` invokes declared
control N. `mod.vector:X:Y` maps pointer position within its hit region to two
slider controls. `mod.drag:X:Y` preserves the initial grab offset and maps pointer
deltas across the authored reference dimensions to the same bounded sliders.
The host owns drawing, input routing and animation; the guest owns feature state.

For a window-free layout capture, run:

```sh
cargo run -p ui-snapshot -- --panel panel.json --out panel.png \
  --width 1280 --height 720 --gui-scale 2 --time 1 --carriers /path/to/compiled
```

The CPU snapshot path uses the production panel painter and local validated
carriers. A headless client capture verifies the GPU glass/backdrop appearance.
