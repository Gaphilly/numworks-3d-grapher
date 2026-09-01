# NumWorks 3D Grapher

NumWorks 3D Grapher is a native Rust `no_std` application for plotting interactive mathematical surfaces on a NumWorks calculator through the EADK. It compiles `z = f(x, y)` expressions into compact postfix bytecode, samples them on a fixed regular grid, and renders wireframe or lit solid views without a heap, GPU, filesystem, general-purpose mesh engine, or full-screen framebuffer.

The project targets `thumbv7em-none-eabihf` (the calculator's Cortex-M7-class Thumb-2 CPU with a hardware single-precision floating-point ABI) and is based on NumWorks' official `epsilon-sample-app-rust` template.

## Current features

- Two surface modes: **Wireframe** and **Solid**
- Three fixed sampling presets: Low 17×13, Standard 25×19, and High 33×25; Standard remains the startup default and released UX/performance baseline
- Cached fixed-world-light Lambert diffuse levels with Standard, Soft, and Strong tone presets
- Ten Solid color choices: Blue, Green, Orange, Purple, Gray, Red, Cyan, Yellow, White, and a user-editable Custom RGB color
- Band-local depth testing for filled triangles
- Configurable ground grid, axes, ticks, coordinate labels, rectangular XY domain, and camera reset
- `f32` expression compiler/evaluator with fixed-arity unary/binary functions, explicit-base logarithms, and safe invalid-domain rejection
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
| Solid | Two directly traversed triangles per regular-grid cell, filled from a cached diffuse-light level and a selected RGB565 appearance table, then ordered by a band-local depth buffer. |

The **Ground grid** setting controls the sparse world-space XY reference grid.

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
| Toolbox | Open the two-column function-template picker |
| Square / Power | Insert `^2` / `^` |
| Left / Right | Move cursor; hold to repeat |
| Shift + Left / Right | Move to start / end |
| Backspace | Delete before cursor; hold to repeat |
| Shift + Backspace (Clear event) | Clear the field |
| EXE | Compile; on success update the surface and return to Graph |
| OK | Give focus to the tab bar |
| Back | Return to Graph without applying edited text |

Toolbox contains `sin`, `cos`, `tan`, `sqrt`, `abs`, `floor`, `ceil`, `round`, `exp`, `ln`, `log`, `min`, `max`, `asin`, `acos`, and `atan`. Up/Down wraps within a column, Left/Right changes columns, EXE inserts the selected template, and Back closes the picker without editing. Unary templates put the cursor inside `()`, while `log(,)`, `min(,)`, and `max(,)` place it after the opening parenthesis.

The edited source and last successfully compiled expression are separate. An invalid edit displays an error but never replaces the active graph.

### Settings content

The eight Settings rows are **Rendering**, **Ground grid**, **Axes**, **Ticks**, **Labels**, **Domain**, **Reset camera**, and **Performance**. Performance toggles the optional latest render-time/FPS readout; it is off by default and does not invalidate graph pixels. On Rendering, Left/Right still switches Wireframe/Solid while EXE opens the Solid Appearance page.

| Input | Action |
| --- | --- |
| Up / Down | Select a row |
| Left / Right | Cycle the rendering mode or set a selected visibility option off/on |
| EXE | Toggle a value, open Appearance/Domain, or reset the camera |
| OK | Give focus to the tab bar |
| Back | Cancel an edit, leave Domain, or return from Settings to Graph |

Rendering and visibility changes only invalidate graph composition. Camera reset restores the established default camera without resampling the surface.

### Appearance page

Appearance provides four bounded Solid-only settings: **Lighting** (`Standard`, `Soft`, or `Strong`), **Surface color** (`Blue`, `Green`, `Orange`, `Purple`, `Gray`, `Red`, `Cyan`, `Yellow`, `White`, or `Custom`), **Resolution** (`Low`, `Standard`, or `High`), and **Auto rotate** (`Off` or `On`). Up/Down selects a row and Left/Right cycles its value. Back returns to the main Settings page. EXE retains forward cycling for built-in colors, opens the Custom Color page when Custom is selected, and toggles Auto rotate.

The Custom Color page edits Red, Green, and Blue channels in `0..=255`. Left/Right adjusts the temporary channel by eight, while EXE opens the existing fixed 24-byte numeric editor for an exact value. Nothing becomes active until EXE on **Apply**; Back cancels a channel draft or the complete Custom Color draft. The page includes an RGB565 preview, uses no heap, and never starts a blocking input loop.

Lighting and color remain independent. Appearance changes invalidate graph composition but do not resample the expression, rebuild triangle normals, alter camera/depth state, or affect Wireframe. Auto rotate is a transient setting and does not change the sampled surface; it advances horizontal yaw only while Graph content has focus and pauses in tabs, Settings, and editors.

### Domain page

Domain exposes `Xmin`, `Xmax`, `Ymin`, and `Ymax`. Up/Down selects a bound, EXE starts editing, and EXE again validates and applies the draft. The editor accepts digits, `.`, `+`, `-`, and scientific-notation `e` through the EE key. Left/Right moves the cursor, Shift + Left/Right moves to the start/end, Backspace deletes, Shift + Backspace clears, and Back cancels the current draft. OK moves to tab focus and also discards an active draft.

Applying a bound is transactional: an invalid draft never changes or resamples the active graph. A complete domain must be finite and ordered, each X/Y span must be at least `0.01` and at most `1000`, and every absolute bound must be at most `1000`. A valid change resamples the active fixed-resolution height cache exactly once.

### Resolution

Appearance also provides **Resolution**. Left/Right cycles **Low** (17×13 points,
384 triangles), **Standard** (25×19 points, 864 triangles), and **High** (33×25
points, 1,536 triangles). Resolution changes resample the surface; lighting and
color changes do not. Standard preserves the original endpoint-inclusive 25×19
sampling arithmetic. Arbitrary and adaptive resolutions are intentionally unsupported.

## Supported expression syntax

Expressions are ASCII and may contain:

- Decimal or scientific-notation `f32` constants, such as `2`, `.5`, and `1e-3`, plus Euler's constant `e`
- Variables `x` and `y`
- Binary operators `+`, `-`, `*`, `/`, and `^`
- Unary minus, parentheses, and whitespace
- Unary functions `sin(...)`, `cos(...)`, `tan(...)`, `sqrt(...)`, `abs(...)`, `floor(...)`, `ceil(...)`, `round(...)`, `exp(...)`, `ln(...)`, `asin(...)`, `acos(...)`, and `atan(...)`
- Binary functions `min(a, b)`, `max(a, b)`, and explicit-base `log(base, value)`

Power and unary negation are right-associative with power binding more tightly: `-2^2` is interpreted as `-(2^2)`, and `2^3^2` as `2^(3^2)`. Function names and variables are lowercase, multiplication must be explicit, and binary-function arguments are comma-separated. `log(base, value)` requires `base > 0`, `base != 1`, and `value > 0`; inverse sine/cosine require an input in `[-1, 1]`. `min` and `max` reject a non-finite operand rather than selecting the other argument. All non-finite evaluation results become invalid samples rather than entering projection or rasterization.

Useful default-domain examples include `abs(x) + abs(y)`, `log(10, x^2 + y^2 + 1)`, `ln(x^2 + y^2 + 1)`, `exp(-(x^2 + y^2))`, `max(x, y)`, `min(x, y)`, and `sin(sqrt(x^2 + y^2))`.

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
active 17×13 / 25×19 / 33×25 cached surface heights
        ↓
orbit-target camera transform + perspective projection
        ├── Wireframe: shared compact screen-point scratch
        └── Solid: the same screen-point scratch + inverse-depth scratch
                    + cached palette-neutral diffuse/validity values
        ↓
regular height-field traversal + bounded axes/grid/labels
        ↓
320×8 RGB565 band rasterizer
        ├── Wireframe: color band only
        └── Solid: color band + u16 inverse-depth band
        ↓
27 EADK graph-viewport transfers
```

The renderer visits only the active cells directly: 16×12 / 24×18 / 32×24 cells, or 384 / 864 / 1,536 consistently wound triangles. It never constructs a general mesh, scene graph, ECS, or other graphics framework. Lambert diffuse light and triangle validity are calculated once when the surface is sampled, never during a camera-only Solid redraw. Three ambient/diffuse curves and nine built-in base colors are combined into 27 compile-time RGB565 tables. Custom RGB uses one persistent 256-entry table that is regenerated only after its RGB value or lighting preset changes. Camera-only redraws reuse the selected table and retain one indexed color lookup per triangle setup with no per-pixel RGB arithmetic or floating-point color work.

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
| Surface cache | Maximum 33×25 capacity: 3,300-byte heights + 232-byte X/Y coordinates + 1,536-byte triangle light/validity cache | Active graph |
| Built-in Solid appearance tables | 27×256×2 = 13,824 bytes | Static flash/rodata; no RAM or stack allocation |
| Active Custom appearance table | 256×2 = 512 bytes, plus a 4-byte cache key | Static writable memory; never placed in `AppState` or the Solid stack frame |
| Shared projected scratch | Maximum 33×25: screen `(i16, i16)` plus Solid-only `u16` inverse depth = 4,950 bytes | Private static RAM, one non-reentrant graph render |
| Expression source | 96 bytes | Equation editor state |
| Domain numeric source | 24 bytes, plus terminator/state | Settings editor state |
| Postfix bytecode | 64 fixed instructions | Active expression |
| Evaluation stack | 32×`f32` = 128 bytes | One sample evaluation |
| Parser operator stack | 32 fixed operators | One compilation |
| Coordinate geometry/labels | 48 lines, 12 labels, at most 12 ticks per axis | One render pass |
| Bitmap label glyphs | 5×7 static flash data for digits/signs and X/Y/Z | Program image |

A full 320×240 RGB565 framebuffer would consume 153,600 bytes, and a full-screen 16-bit depth buffer would consume another 153,600 bytes. Both are avoided. The graph viewport is 216 pixels high below its 24-pixel header, so the renderer composes exactly 27 eight-row bands and performs one EADK rectangle transfer per band—never one firmware call per pixel.

Wireframe and Solid retain separate render call paths. They share only a renderer-private persistent projection scratch area so High resolution does not grow the render stack. Wireframe writes and reads only screen coordinates and never consumes Solid depth data; it still avoids the Solid-only 5,120-byte depth band. The scratch is safe only under the cooperative non-reentrant render loop and is fully populated before rendering borrows it immutably.

### Stack budget

The current `release-extreme` ARM disassembly shows an approximately **1.4 KiB** `main` frame, a 72-byte Solid wrapper, and an approximately **11.3 KiB** Solid composition closure: about **12.8 KiB** of directly observable application frames before deeper helper/firmware calls. The maximum surface cache and projection scratch deliberately live in persistent BSS instead of these frames. This is a stack **risk budget**, not a formal safety guarantee. EADK firmware-call depth, interrupt nesting, and the stack range assigned to an external app are firmware/device dependent and are not exposed by the public EADK ABI. Do not increase Solid-path stack storage without repeating the ARM frame analysis and validating on real hardware.

## Rendering pipeline

Every graph redraw uses one immutable camera state. It projects the cached surface once, builds bounded coordinate geometry, waits for vertical blank, and produces the 27 graph bands from top to bottom. Each band uses this composition order:

1. Clear to the graph background.
2. Draw the world-space XY ground grid.
3. Draw world axes.
4. Draw coordinate-label background rectangles.
5. Draw numeric 5×7 bitmap labels.
6. Draw the wireframe or depth-tested solid surface.
8. Draw tick marks and the origin.
9. Draw X/Y/Z bitmap labels.
10. Push the completed band with EADK.

Numeric labels precede the surface and can be covered by it; ticks, the origin, and axis names are deliberately composed afterward for readability. Graph labels remain inside the band buffer. Do not replace them with `eadk_display_draw_string()` or draw directly to the display between band transfers: doing so can expose stale labels during camera redraws and was the cause of a previous flashing/alternating-frame artifact. Firmware text drawing remains appropriate for independently redrawn header, Equation, and Settings UI regions.

Solid projection encodes normalized reciprocal camera-space depth into `u16`: zero is invalid and larger values are nearer. Reciprocal depth is stepped across each triangle together with incremental integer edge equations, so covered pixels avoid repeated edge evaluations and per-pixel division.

## Known rendering limitations

- Fixed grids can miss detail or a discontinuity that falls entirely between sample positions; Low is coarser and High is denser, with the expected performance tradeoff.
- Triangles touching a NaN/infinite sample, invalid projection, degenerate normal, or implausibly large finite height jump are omitted. The local-jump rule prevents many pole-spanning triangles but is intentionally heuristic: it can remove a legitimate very steep patch or miss an unsampled pole.
- Auxiliary world-space lines are clipped against the near plane. Wireframe surface edges with an invalid projected endpoint are omitted, and solid triangles with any invalid projected vertex are rejected instead of being polygon-clipped, so moving the camera through the surface can create temporary holes or popping.
- A 16-bit inverse-depth band is a deliberate memory/precision tradeoff. Host tests cover ordering, ties, extremes, and interpolation, but only hardware inspection can determine whether quantization is visually acceptable. The bounded fallback is a solid-only `u32` band, never a full-screen depth buffer.
- Numeric label placement is conservative and the final ticks/axis glyphs are composed for readability rather than full per-pixel text occlusion.

## Hardware performance and visual validation

Real NumWorks hardware is the performance and UX target. Host timings are not evidence that Solid is fast enough, and automated tests cannot establish perceived lighting quality, surface readability, grid/axis composition, depth artifacts, label readability, camera-motion smoothness, or overall usability.

The application records the complete graph render—including vertical-blank waiting and display transfers—in every build. The Settings tab's **Performance** option controls whether the latest `Last:…ms …FPS` readout is shown; it is off by default. The optional `RENDER_FREEZE_DIAGNOSTICS` flag in `src/main.rs` enables a small Solid-render phase/band breadcrumb for future hardware investigations; it is off in the release configuration.

Before sign-off, install with `cargo run` and test both rendering modes at identical domains and camera states with:

- `sin(x) * cos(y)` — baseline composition and lighting
- `x^2 + y^2` — bowl depth and gradients
- `x^2 - y^2` — saddle readability
- `sin(sqrt(x^2 + y^2))` — radial detail
- `sqrt(x)` — open invalid negative-X region
- `1/x` — no triangles bridging the pole
- `tan(x)` — discontinuity gaps without screen-sized spikes
- `1/(x*y)` — invalid axes and four separated regions without bridging

For each expression, orbit, pan, dolly, and change FOV while checking lighting, shape readability, ground-grid/axis composition, depth artifacts, labels, continuous-camera smoothness, and invalid-region behavior. Compare Wireframe directly with the released 2.0.0 baseline experience.

If a technically correct feature makes the calculator noticeably less responsive, responsiveness wins: optimize incremental rasterization and rejection before accepting an interaction regression. Do not alter the wireframe renderer to hide solid-mode performance costs.

## Project structure

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | NWA metadata, exported entry point, cooperative input/render loop, surface-cache ownership, render timing, and optional freeze diagnostics |
| `src/app.rs` | Tab/content focus state machine, graph options, domain state, and header/content/graph/surface dirty flags |
| `src/eadk.rs` | Rust FFI layout, constants, and guarded wrappers for firmware display, keyboard, event, and timing symbols |
| `src/editor.rs` | Fixed 96-byte Equation editor, cursor/scroll logic, bounded Toolbox function picker, calculator event mapping, and held-key repeat |
| `src/settings.rs` | Eight-row Settings interaction, Appearance/transactional Custom RGB pages, optional performance readout, and shared fixed 24-byte numeric editor |
| `src/expression.rs` | Streaming tokenizer, shunting-yard parser, fixed postfix bytecode, and stack evaluator |
| `src/function.rs` | `SurfaceFunction` boundary between evaluation and sampling |
| `src/surface.rs` | Domain validation/mapping, fixed Low/Standard/High sampling, cached heights/coordinates/diffuse levels, and discontinuity rejection |
| `src/camera.rs` | Orbit target/distance state, translation, perspective projection, and near-plane line clipping |
| `src/graph.rs` | Rendering/lighting/palette options, central coordinate colors, domain-aware axis visibility, and bounded 1/2/5 ticks |
| `src/rendering.rs` | Wireframe/solid projected caches, coordinate/label geometry, band rasterization, inverse-depth testing, and EADK transfers |
| `src/input.rs` | Smooth raw-key graph camera mapping |
| `src/ui.rs` | Tab header, Equation UI, Settings/Appearance/domain UI, version display, and user-facing errors |
| `src/math.rs` | Small `no_std` `f32` trigonometric, inverse-trigonometric, logarithmic, root, rounding, and power approximations |
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

Settings displays the manually maintained version string **`v2.6.1`**. Routine renderer, documentation, build, or packaging changes must not modify it; update it only when an explicit version change is requested. It is independent of automatic timestamps or generated artifacts.

## v2.6.1 changes

- Adds optional horizontal automatic camera rotation from Appearance settings.
- Auto-rotation is frame-rate independent, pauses outside Graph content focus, and preserves surface sampling and rendering performance.

## v2.6.0 changes

- Adds `e`, `floor`, `ceil`, `round`, `exp`, `ln`, `min`, `max`, explicit-base `log(base, value)`, `asin`, `acos`, and `atan` to the fixed-capacity evaluator.
- Adds fixed-arity comma parsing with safe nested calls and structured invalid-argument errors.
- Replaces the direct Toolbox `abs()` shortcut with a compact two-column function-template picker.
- Preserves the released Wireframe and Solid render paths, surface layout, display-transfer strategy, and color/depth architecture.
- Adds Low (17×13), Standard (25×19), and High (33×25) fixed resolution presets; Standard remains the released baseline.
- Keeps the maximum-capacity sampled surface and projection scratch in persistent fixed storage so resolution changes do not grow the render stack.
- Sets the manually maintained displayed/package version to 2.6.0 for this release.

## v2.4.0 changes

- Expands Solid colors to nine built-in RGB565 choices plus a transactional Custom RGB color.
- Reuses the existing semantic numeric editor for exact `0..=255` channel entry and provides a bounded preview/apply workflow.
- Adds 27 compile-time built-in shade tables and one persistent 512-byte Custom table, rebuilt only when Custom RGB or lighting changes.
- Keeps cached triangle lighting palette-neutral and leaves sampling, projection, depth, the Solid pixel loop, and Wireframe unchanged.
- Sets the manually maintained displayed/package version to 2.4.0 for this release.

## v2.3.0 changes

- Adds Solid lighting presets: Standard, Soft, and Strong.
- Adds selectable Solid surface palettes: Blue, Green, Orange, Purple, and Gray.
- Adds a compact Appearance page under Settings while preserving the existing Settings navigation model.
- Keeps lighting cached per triangle and color selection table-based, so camera-only Solid redraws avoid new per-pixel color work.
- Preserves the released Wireframe renderer and keeps the project version manually maintained.

## v2.2.0 changes

- Caches exact sampled X/Y coordinates for the Solid projection path, removing repeated domain-coordinate divisions during camera-only redraws.
- Caches the 864 Solid triangle light/validity values when a surface is sampled or resampled, removing normal construction, square roots, and divisions from camera-only Solid redraws.
- Preserves the released Wireframe projection/render path; Wireframe does not read the new Solid caches.
- Adds a reference-equivalence test for cached coordinates and triangle lighting, and documents the 1,040-byte active-surface cache extension.

## v2.1.0 changes

- Removed the unstable Solid + Grid mode. Rendering now offers the proven Wireframe baseline and depth-tested Solid mode only.
- Preserved Wireframe's separate 25×19, 27-band path and its released controls/composition.
- Added an optional Settings performance readout showing the latest complete render duration and derived FPS in every build.
- Added an opt-in, compile-time Solid freeze breadcrumb (`RENDER_FREEZE_DIAGNOSTICS`) for hardware diagnosis; it is disabled in the release configuration.
- Retained allocation-free, `no_std` operation with the existing 320×8 RGB565 band buffers and no full-screen framebuffer or depth buffer.

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
