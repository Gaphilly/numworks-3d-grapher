# NumWorks 3D Grapher

NumWorks 3D Grapher is a native Rust `no_std` application for plotting interactive mathematical surfaces on a NumWorks calculator through the EADK. It compiles `z = f(x, y)` expressions into compact postfix bytecode, samples them on a fixed regular grid, and renders wireframe or lit solid views without a heap, GPU, filesystem, general-purpose mesh engine, or full-screen framebuffer.

The project targets `thumbv7em-none-eabihf` (the calculator's Cortex-M7-class Thumb-2 CPU with a hardware single-precision floating-point ABI) and is based on NumWorks' official `epsilon-sample-app-rust` template.

## Current features

- Three surface modes: **Wireframe**, **Solid**, and **Solid + Grid**
- The original 25×19 wireframe renderer remains the startup default and UX/performance baseline
- Ambient plus fixed-world-light Lambert shading for 864 regular height-field triangles
- Band-local depth testing for filled triangles; Solid + Grid reuses the solid projection, fill, and depth results
- Configurable ground grid, axes, ticks, coordinate labels, rectangular XY domain, and camera reset
- `f32` expression compiler/evaluator supporting constants, `x`, `y`, arithmetic, power, parentheses, unary minus, and `sin`, `cos`, `tan`, `sqrt`, and `abs`
- Calculator-style Equation editor with a 96-byte buffer, cursor, scrolling, shortcuts, errors, and held-key repeat
- Transactional 24-byte numeric editor for each domain bound
- Graph, Equation, and Settings tabs with explicit focus and separate graph/surface dirty state
- Orbit, truck/track, pedestal, true dolly, and independent perspective/FOV controls
- Fixed-capacity, deterministic, no-heap architecture designed for the calculator's limited RAM and ARM CPU
- Host-side tests for parsing, editing, state transitions, camera math, sampling, projection, triangle lighting/depth, ticks, and labels

Settings are retained only for the current application session; there is no persistent storage.

## Rendering modes

| Mode | Behavior |
| --- | --- |
| Wireframe | The established low-cost row/column mesh. It has its own render path and does not allocate or calculate solid-only depth or lighting state. |
| Solid | Two directly traversed triangles per regular-grid cell, filled with one ambient-plus-Lambert shade per triangle and ordered by a band-local depth buffer. |
| Solid + Grid | The Solid result plus a depth-tested surface-grid overlay. It reuses the same samples, projection, triangle fill, and depth band; no second fill or projection pass is performed. |

The **Ground grid** setting controls the sparse world-space XY reference grid. It is separate from the surface mesh drawn by **Solid + Grid**.

## Controls

### Graph content

| Input | Action |
| --- | --- |
| Left / Right | Orbit yaw around the current target |
| Up / Down | Orbit pitch around the current target |
| Shift + Left / Right | Truck/track target and camera horizontally relative to current yaw |
| Shift + Up / Down | Pedestal target and camera along world Z |
| `+` / `-` | Dolly toward / away from the target (true camera distance) |
| Alpha + `+` / `-` | Increase / decrease focal length (narrower / wider FOV) |
| OK | Give focus to the tab bar |
| Back | Exit the application |

Camera keys use raw keyboard state and repeat smoothly while held. Distance, pitch, focal length, and translations are clamped to finite practical ranges. Camera movement reprojects the existing samples but does not reevaluate the expression.

### Tab bar

| Input | Action |
| --- | --- |
| Left / Right | Highlight the previous / next tab |
| OK | Activate the highlighted tab and focus its content |
| Back | Cancel tab navigation and return to the current content |

### Equation content

| Input | Action |
| --- | --- |
| Calculator digits/operators/parentheses | Insert the corresponding expression character |
| XNT | Insert `x` |
| Alpha letters | Insert lowercase letters, including `y` |
| sin, cos, tan, sqrt | Insert a complete function template with the cursor inside `()` |
| Toolbox | Insert `abs()` with the cursor inside |
| Square / Power | Insert `^2` / `^` |
| Left / Right | Move cursor; hold to repeat |
| Shift + Left / Right | Move to start / end |
| Backspace | Delete before cursor; hold to repeat |
| Shift + Backspace (Clear event) | Clear the field |
| EXE | Compile; on success update the surface and return to Graph |
| OK | Give focus to the tab bar |
| Back | Return to Graph without applying edited text |

The edited source and last successfully compiled expression are separate. An invalid edit displays an error but never replaces the active graph.

### Settings content

The seven Settings rows are **Rendering**, **Ground grid**, **Axes**, **Ticks**, **Labels**, **Domain**, and **Reset camera**.

| Input | Action |
| --- | --- |
| Up / Down | Select a row |
| Left / Right | Cycle the rendering mode or set a selected visibility option off/on |
| EXE | Cycle/toggle a value, open Domain, or reset the camera |
| OK | Give focus to the tab bar |
| Back | Cancel an edit, leave Domain, or return from Settings to Graph |

Rendering and visibility changes only invalidate graph composition. Camera reset restores the established default camera without resampling the surface.

### Domain page

Domain exposes `Xmin`, `Xmax`, `Ymin`, and `Ymax`. Up/Down selects a bound, EXE starts editing, and EXE again validates and applies the draft. The editor accepts digits, `.`, `+`, `-`, and scientific-notation `e` through the EE key. Left/Right moves the cursor, Shift + Left/Right moves to the start/end, Backspace deletes, Shift + Backspace clears, and Back cancels the current draft. OK moves to tab focus and also discards an active draft.

Applying a bound is transactional: an invalid draft never changes or resamples the active graph. A complete domain must be finite and ordered, each X/Y span must be at least `0.01` and at most `1000`, and every absolute bound must be at most `1000`. A valid change resamples the 25×19 height cache exactly once.

## Supported expression syntax

Expressions are ASCII and may contain:

- Decimal or scientific-notation `f32` constants, such as `2`, `.5`, and `1e-3`
- Variables `x` and `y`
- Binary operators `+`, `-`, `*`, `/`, and `^`
- Unary minus, parentheses, and whitespace
- Functions `sin(...)`, `cos(...)`, `tan(...)`, `sqrt(...)`, and `abs(...)`

Power and unary negation are right-associative with power binding more tightly: `-2^2` is interpreted as `-(2^2)`, and `2^3^2` as `2^(3^2)`. Function names and variables are lowercase, and multiplication must be explicit. Non-finite evaluation results become invalid samples rather than entering projection or rasterization.

## Architecture

The mathematical/rendering path is:

```text
Expression text (96 bytes)
        ↓
streaming tokenizer + shunting-yard parser
        ↓
fixed postfix bytecode (64 instructions)
        ↓
fixed-stack f32 evaluator
        ↓
25×19 cached surface heights
        ↓
orbit-target camera transform + perspective projection
        ├── Wireframe: compact screen-point cache
        └── Solid: compact screen point + inverse-depth cache
                    + 864 transient triangle shades
        ↓
regular height-field traversal + bounded axes/grid/labels
        ↓
320×8 RGB565 band rasterizer
        ├── Wireframe: color band only
        └── Solid: color band + u16 inverse-depth band
        ↓
27 EADK graph-viewport transfers
```

The renderer visits the 24×18 cells directly as 864 consistently wound triangles; it never constructs a general mesh, scene graph, ECS, or other graphics framework. Ambient-plus-Lambert lighting and triangle validity are calculated once per triangle per solid redraw, outside the 27-band loop. Solid + Grid traverses each unique horizontal or vertical height-field edge once per band topology pass (906 edges), reusing the completed fill depth instead of repeating expensive surface work.

The input/UI path is deliberately split:

```text
eadk_keyboard_scan() ──→ raw edges ──→ tab/focus/settings state machines
          │
          └────────────→ held state ─→ continuous camera + shared editor repeat

bounded eadk_event_get() ────────────→ semantic Equation/domain characters

AppState dirty flags ────────────────→ header / content / graph / surface redraws
```

Raw state owns continuous motion and application-level focus. Semantic events are polled only after a raw down edge in tab/editor contexts; there is no blocking editor loop. Camera and appearance changes invalidate projection/composition but reuse cached heights. Only a successful equation compile or validated domain change resamples the surface. The graph dirty flag remains set while another tab covers the viewport so the changed graph is composed when Graph becomes visible again.

## Memory model

There is no allocator and no heap-backed collection. Rendering storage is fixed-capacity and deterministic.

| Storage | Size / capacity | Lifetime |
| --- | ---: | --- |
| RGB565 color band | 320×8×2 = 5,120 bytes | Every graph render path |
| Solid inverse-depth band | 320×8×2 = 5,120 bytes | Solid modes only |
| Cached surface heights | 25×19 `f32` = 1,900 bytes, plus range metadata | Active graph |
| Wireframe projected cache | 25×19 `(i16, i16)` = 1,900 bytes | Wireframe render only |
| Solid projected cache | 25×19 `(i16, i16, u16)` = approximately 2,850 bytes | Solid render only |
| Triangle light/validity cache | 24×18×2 `u8` = 864 bytes | Solid render only |
| Expression source | 96 bytes | Equation editor state |
| Domain numeric source | 24 bytes, plus terminator/state | Settings editor state |
| Postfix bytecode | 64 fixed instructions | Active expression |
| Evaluation stack | 32×`f32` = 128 bytes | One sample evaluation |
| Parser operator stack | 32 fixed operators | One compilation |
| Coordinate geometry/labels | 48 lines, 12 labels, at most 12 ticks per axis | One render pass |
| Bitmap label glyphs | 5×7 static flash data for digits/signs and X/Y/Z | Program image |

A full 320×240 RGB565 framebuffer would consume 153,600 bytes, and a full-screen 16-bit depth buffer would consume another 153,600 bytes. Both are avoided. The graph viewport is 216 pixels high below its 24-pixel header, so the renderer composes exactly 27 eight-row bands and performs one EADK rectangle transfer per band—never one firmware call per pixel.

Wireframe and solid use separate call paths. Wireframe therefore retains its established color-band and projected-cache footprint and does not reserve the solid-only 5,120-byte depth band, approximately 2,850-byte projected vertex array, or 864-byte lighting array.

### Stack budget

The current `release-extreme` ARM disassembly confirms an approximately **20.5 KiB** application-side peak on the Solid path: the long-lived `main` frame, `render_solid`, and world-geometry construction are simultaneously live. This is a stack **risk budget**, not a formal safety guarantee. EADK firmware-call depth, interrupt nesting, and the stack range assigned to an external app are firmware/device dependent and are not exposed by the public EADK ABI. Do not increase Solid-path stack storage without repeating the ARM frame analysis and validating on real hardware.

## Rendering pipeline

Every graph redraw uses one immutable camera state. It projects the cached surface once, builds bounded coordinate geometry, waits for vertical blank, and produces the 27 graph bands from top to bottom. Each band uses this composition order:

1. Clear to the graph background.
2. Draw the world-space XY ground grid.
3. Draw world axes.
4. Draw coordinate-label background rectangles.
5. Draw numeric 5×7 bitmap labels.
6. Draw the wireframe or depth-tested solid surface.
7. In Solid + Grid, draw the depth-tested surface-grid overlay.
8. Draw tick marks and the origin.
9. Draw X/Y/Z bitmap labels.
10. Push the completed band with EADK.

Numeric labels precede the surface and can be covered by it; ticks, the origin, and axis names are deliberately composed afterward for readability. Graph labels remain inside the band buffer. Do not replace them with `eadk_display_draw_string()` or draw directly to the display between band transfers: doing so can expose stale labels during camera redraws and was the cause of a previous flashing/alternating-frame artifact. Firmware text drawing remains appropriate for independently redrawn header, Equation, and Settings UI regions.

Solid projection encodes normalized reciprocal camera-space depth into `u16`: zero is invalid and larger values are nearer. Reciprocal depth is stepped across each triangle together with incremental integer edge equations, so covered pixels avoid repeated edge evaluations and per-pixel division. Solid + Grid compares its interpolated edge depth with the completed fill depth and does not expose hidden back-facing edges.

## Known rendering limitations

- The fixed 25×19 sample grid can miss detail or a discontinuity that falls entirely between sample positions.
- Triangles touching a NaN/infinite sample, invalid projection, degenerate normal, or implausibly large finite height jump are omitted. The local-jump rule prevents many pole-spanning triangles but is intentionally heuristic: it can remove a legitimate very steep patch or miss an unsampled pole.
- Auxiliary world-space lines are clipped against the near plane. Wireframe surface edges with an invalid projected endpoint are omitted, and solid triangles with any invalid projected vertex are rejected instead of being polygon-clipped, so moving the camera through the surface can create temporary holes or popping.
- A 16-bit inverse-depth band is a deliberate memory/precision tradeoff. Host tests cover ordering, ties, extremes, and interpolation, but only hardware inspection can determine whether quantization is visually acceptable. The bounded fallback is a solid-only `u32` band, never a full-screen depth buffer.
- Numeric label placement is conservative and the final ticks/axis glyphs are composed for readability rather than full per-pixel text occlusion.

## Hardware performance and visual validation

Real NumWorks hardware is the performance and UX target. Host timings are not evidence that Solid or Solid + Grid is fast enough, and automated tests cannot establish perceived lighting quality, surface readability, grid/axis composition, depth artifacts, label readability, camera-motion smoothness, or overall usability.

**Hardware sign-off for this solid-rendering milestone is still pending.** Building and host tests validate integration but do not complete the milestone. A debug build temporarily measures the complete graph render—including vertical-blank waiting and display transfers—and shows `Last:…ms` in Settings. Release builds contain neither that field nor the profiling readout.

Before sign-off, install with `cargo run` and test all three rendering modes at identical domains and camera states with:

- `sin(x) * cos(y)` — baseline composition and lighting
- `x^2 + y^2` — bowl depth and gradients
- `x^2 - y^2` — saddle readability
- `sin(sqrt(x^2 + y^2))` — radial detail and surface-grid clarity
- `sqrt(x)` — open invalid negative-X region
- `1/x` — no triangles bridging the pole
- `tan(x)` — discontinuity gaps without screen-sized spikes
- `1/(x*y)` — invalid axes and four separated regions without bridging

For each expression, orbit, pan, dolly, and change FOV while checking lighting, shape readability, ground-grid/axis composition, depth artifacts, labels, continuous-camera smoothness, and invalid-region behavior. Compare Wireframe directly with the released 2.0.0 baseline experience.

Solid + Grid is acceptable only when it adds no noticeable control lag and costs no more than approximately `max(5 ms, 25%)` over Solid at the default view. If a technically correct feature makes the calculator noticeably less responsive, responsiveness wins: optimize incremental rasterization and rejection first, eliminate repeated work, reduce overlay density if necessary, and simplify lighting/overlay detail before accepting an interaction regression. Do not alter the wireframe renderer to hide solid-mode performance costs.

## Project structure

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | NWA metadata, exported entry point, cooperative input/render loop, surface-cache ownership, and debug render timing |
| `src/app.rs` | Tab/content focus state machine, graph options, domain state, and header/content/graph/surface dirty flags |
| `src/eadk.rs` | Rust FFI layout, constants, and guarded wrappers for firmware display, keyboard, event, and timing symbols |
| `src/editor.rs` | Fixed 96-byte Equation editor, cursor/scroll logic, calculator event mapping, and held-key repeat |
| `src/settings.rs` | Seven-row Settings interaction and fixed 24-byte transactional domain editor |
| `src/expression.rs` | Streaming tokenizer, shunting-yard parser, fixed postfix bytecode, and stack evaluator |
| `src/function.rs` | `SurfaceFunction` boundary between evaluation and sampling |
| `src/surface.rs` | Domain validation/mapping, 25×19 sampling, cached heights, discontinuity rejection, and transient triangle lighting |
| `src/camera.rs` | Orbit target/distance state, translation, perspective projection, and near-plane line clipping |
| `src/graph.rs` | Rendering mode/options, central RGB565 palette, domain-aware axis visibility, and bounded 1/2/5 ticks |
| `src/rendering.rs` | Wireframe/solid projected caches, coordinate/label geometry, band rasterization, inverse-depth testing, and EADK transfers |
| `src/input.rs` | Smooth raw-key graph camera mapping |
| `src/ui.rs` | Tab header, Equation UI, Settings/domain UI, version display, and user-facing errors |
| `src/math.rs` | Small `no_std` `f32` trigonometric, root, and power approximations |
| `src/icon.png` | Application icon source converted to NWI during embedded builds |
| `build.rs` | Host-side timestamped PNG-to-NWI conversion through nwlink |
| `.cargo/config.toml` | Default ARM target, relocatable linker settings, and pinned install runner |

## Development

Prerequisites are Rust/rustup, Node.js/npm, nwlink-compatible USB access, and a supported NumWorks calculator. Install the embedded target once:

```bash
rustup target add thumbv7em-none-eabihf
```

Build debug, standard release, or the size/performance-oriented `release-extreme` embedded relocatable application:

```bash
cargo build
cargo build --release
cargo build --profile release-extreme
```

`release-extreme` inherits `release` and uses `opt-level = 3`, fat LTO, one codegen unit, `panic = "abort"`, stripped symbols, and no incremental compilation. It is useful for final embedded inspection, but its generated frame layout can differ from the debug or ordinary release build.

`build.rs` converts `src/icon.png` to `target/icon.nwi` when needed. EADK symbols remain unresolved at this partial-link stage because their implementations live in calculator firmware.

Connect the calculator over USB and build/install with the configured nwlink 0.0.19 runner:

```bash
cargo run
```

Unlike `cargo build`, `cargo run` invokes `npx --yes -- nwlink@0.0.19 install-nwa`; nwlink consumes the relocatable application, resolves EADK imports through the external-app trampoline mechanism, packages it as an NWA, and installs it.

Run logic tests on the host explicitly because the Cargo default target is embedded:

```bash
cargo test --target x86_64-unknown-linux-gnu
```

The complete pre-handoff check is:

```bash
cargo fmt -- --check
cargo test --target x86_64-unknown-linux-gnu
cargo build
cargo build --release
git diff --check
cargo run
```

The final command requires a connected calculator and begins the mandatory physical visual/performance checklist above.

## Version policy

Settings displays the manually maintained version string **`v2.0.0`**. Routine renderer, documentation, build, or packaging changes must not modify it; update it only when an explicit version change is requested. It is independent of automatic timestamps or generated artifacts.

## Contributing and development rules

- Keep target code `no_std`, allocation-free, fixed-capacity, and deterministic unless a deliberate measured design change justifies otherwise.
- Respect capacities, validate indices/pointer lengths before FFI, and preserve EADK C ABI layouts.
- Prefer `f32`; reject NaN/infinity before converting to screen coordinates.
- Do not add a full-screen color/depth buffer, per-pixel EADK calls, general graphics engine, scene graph, ECS, allocator, or mesh framework.
- Preserve the wireframe UX/performance baseline, graph band composition, label readability, and raw/semantic input ownership split.
- Add host tests for non-trivial parser, editor, state, geometry, camera, depth, or numerical logic.
- Treat physical calculator testing as mandatory for rendering/input changes and performance claims.
- For every significant subsystem change, document its public API, invariants, memory/performance impact, and user-visible controls here.

## License and trademarks

This project retains the template's BSD license; see [LICENSE](LICENSE). NumWorks and Rust are trademarks of their respective owners.
