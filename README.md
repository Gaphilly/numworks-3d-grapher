# NumWorks 3D Grapher

NumWorks 3D Grapher is a native Rust `no_std` application for plotting interactive mathematical surfaces on a NumWorks calculator through the EADK. It independently compiles up to four `z = f(x, y)` expressions into compact postfix bytecode, samples them on one shared regular grid, and renders simultaneous wireframe or depth-tested solid views without a heap, GPU, filesystem, general-purpose mesh engine, or full-screen framebuffer.

The project targets `thumbv7em-none-eabihf` (the calculator's Cortex-M7-class Thumb-2 CPU with a hardware single-precision floating-point ABI) and is based on NumWorks' official `epsilon-sample-app-rust` template.

## Current features

- Two surface modes: **Wireframe** and **Solid**
- Four independently enabled functions with transactional drafts and per-function colors
- Bounded numerical intersection detection for all six function pairs, with visibility toggles, counts, and representative coordinates
- Four fixed sampling presets: Low 17×13, Standard 25×19, High 33×25, and Ultra 41×31; Standard remains the startup default and released UX/performance baseline
- Cached fixed-world-light Lambert diffuse levels with Standard, Soft, and Strong tone presets
- Ten per-function color choices: Blue, Green, Orange, Purple, Gray, Red, Cyan, Yellow, White, and a user-editable Custom RGB color
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

Equation opens a four-row function list. Up/Down selects F1–F4, EXE opens its detail page, Toolbox opens the six-pair Intersections page, Back returns to Graph, and OK focuses the tab bar. Each detail page controls **Enabled**, **Expression**, and **Color**. An empty function cannot be enabled; attempting to do so opens its editor. Each function keeps its own last valid bytecode and independent 96-byte draft.

The Intersections page lists F1/F2, F1/F3, F1/F4, F2/F3, F2/F4, and F3/F4 in deterministic order. Left/Right or EXE toggles only graph visibility; detection remains cached so counts stay available. `256+` means the bounded display cache was truncated. The selected pair shows one deterministic approximate `(x,y,z)` representative when geometry exists.

Inside a selected function's expression editor:

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
| EXE | Compile selected function; on success update only its surface/intersections and return to Graph |
| OK | Give focus to the tab bar |
| Back | Return to Graph without applying edited text |

Toolbox contains `sin`, `cos`, `tan`, `sqrt`, `abs`, `floor`, `ceil`, `round`, `exp`, `ln`, `log`, `min`, `max`, `asin`, `acos`, and `atan`. Up/Down wraps within a column, Left/Right changes columns, EXE inserts the selected template, and Back closes the picker without editing. Unary templates put the cursor inside `()`, while `log(,)`, `min(,)`, and `max(,)` place it after the opening parenthesis.

The edited source and last successfully compiled expression are separate for every slot. An invalid edit displays an error but never replaces active bytecode, sampled geometry, or cached intersections. Back preserves the unapplied draft and returns to function detail.

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

Appearance provides three bounded global settings: **Lighting** (`Standard`, `Soft`, or `Strong`), **Resolution** (`Low`, `Standard`, `High`, or `Ultra`), and **Auto rotate** (`Off` or `On`). Up/Down selects a row and Left/Right or EXE cycles its value. Back returns to the main Settings page.

Surface color now belongs to each function's detail page. Built-in colors cycle directly; EXE on Custom opens a transactional Red/Green/Blue editor. Left/Right adjusts the temporary channel by eight, while EXE on a channel opens the shared fixed 24-byte numeric editor for an exact `0..=255` value. Nothing becomes active until EXE on **Apply**; Back cancels the numeric edit or complete RGB draft.

Lighting and function colors remain independent. Their changes invalidate graph composition but do not resample expressions, rebuild triangle normals, alter camera/depth state, or change intersection detection. Auto rotate remains camera-only and pauses whenever Graph content lacks focus.

### Domain page

Domain exposes `Xmin`, `Xmax`, `Ymin`, and `Ymax`. Up/Down selects a bound, EXE starts editing, and EXE again validates and applies the draft. The editor accepts digits, `.`, `+`, `-`, and scientific-notation `e` through the EE key. Left/Right moves the cursor, Shift + Left/Right moves to the start/end, Backspace deletes, Shift + Backspace clears, and Back cancels the current draft. OK moves to tab focus and also discards an active draft.

Applying a bound is transactional: an invalid draft never changes or resamples the active graph. A complete domain must be finite and ordered, each X/Y span must be at least `0.01` and at most `1000`, and every absolute bound must be at most `1000`. A valid change resamples the active fixed-resolution height cache exactly once.

### Resolution

Appearance also provides **Resolution**. Left/Right cycles **Low** (17×13 points,
384 triangles), **Standard** (25×19 points, 864 triangles), **High** (33×25
points, 1,536 triangles), and **Ultra** (41×31 points, 2,400 triangles).
Resolution changes resample the surface; lighting and color changes do not.
Standard preserves the original endpoint-inclusive 25×19 sampling arithmetic.
Ultra uses larger persistent fixed-capacity surface/projection caches, not stack
arrays. Arbitrary and adaptive resolutions are intentionally unsupported.

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
four fixed postfix programs (64 instructions each)
        ↓
fixed-stack f32 evaluator
        ↓
shared X/Y samples + four independent fixed-capacity height/light caches
        ↓
six bounded marching-triangle intersection caches
        ↓
orbit-target camera transform + perspective projection
        ├── Wireframe: four compact screen-point slots
        └── Solid: four screen-point + inverse-depth slots
                    sharing one band-local depth buffer
        ↓
regular height-field traversal + bounded axes/grid/labels
        ↓
320×8 RGB565 band rasterizer
        ├── Wireframe: color band only
        └── Solid: color band + u16 inverse-depth band
        ↓
27 EADK graph-viewport transfers
```

Each enabled function directly traverses only its active cells: 16×12, 24×18, 32×24, or 40×30. All Solid triangles write into the same 320×8 depth band, so nearer geometry wins across functions without separate complete-surface renders. Lambert diffuse light and validity are cached per function at sampling time. Built-ins use the existing 27 flash tables; Custom uses one persistent 256-entry table per function. Camera-only redraws perform no expression evaluation, relighting, intersection solving, or color generation.

Intersections use camera-independent marching triangles over `f-g`. Every pair is bounded to 256 stored segments/markers, invalid or discontinuous triangles are rejected through both surfaces' cached validity, and overflow is selected deterministically across row-major candidates. Exact sampled tangencies can produce point markers; an unsampled tangent may be missed.

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
| Shared surface bank | 30,276 bytes linked: one shared X/Y coordinate cache plus four 7,496-byte height/light/range caches | Persistent active graph |
| Built-in Solid appearance tables | 27×256×2 = 13,824 bytes | Static flash/rodata; no RAM or stack allocation |
| Per-function Custom tables | 4×256×2 = 2,048 bytes, plus four cache keys | Static writable memory; never placed on the Solid stack |
| Surface projection bank | 4×7,626 = 30,504 bytes | Private static RAM, one non-reentrant graph render |
| Projected intersection bank | 6×256×12 plus counts = 18,444 bytes | Same private projection bank |
| Packed intersection cache | Six pairs × 256 eight-byte segments plus metadata = 12,340 bytes linked | Persistent camera-independent geometry |
| Function slots | 2,480 bytes linked for four compiled programs, drafts, state, and colors | Persistent application state |
| Reusable expression source | 96 bytes | One Equation editor loaded from the selected slot |
| Domain numeric source | 24 bytes, plus terminator/state | Settings editor state |
| Postfix bytecode | 64 fixed instructions | Active expression |
| Evaluation stack | 32×`f32` = 128 bytes | One sample evaluation |
| Parser operator stack | 32 fixed operators | One compilation |
| Coordinate geometry/labels | 48 lines, 12 labels, at most 12 ticks per axis | One render pass |
| Bitmap label glyphs | 5×7 static flash data for digits/signs and X/Y/Z | Program image |

A full 320×240 RGB565 framebuffer would consume 153,600 bytes, and a full-screen 16-bit depth buffer would consume another 153,600 bytes. Both are avoided. The graph viewport is 216 pixels high below its 24-pixel header, so the renderer composes exactly 27 eight-row bands and performs one EADK rectangle transfer per band—never one firmware call per pixel.

Wireframe and Solid retain separate render call paths. Their private projection bank is populated for every enabled function before an immutable reference reaches geometry/rasterization. No mutable projection reference coexists with raster reads, and rendering is single-threaded/non-reentrant. Wireframe consumes only screen coordinates and still avoids the Solid-only 5,120-byte depth band.

### Stack budget

The current implementation artifact links **93,612 bytes of `.bss`** and **2,496 bytes of `.data`**, versus 15,922/4 bytes in v2.7.0. ARM disassembly shows a 640-byte `main` local allocation, a 32-byte graph-dispatch helper, and an approximately 11.2 KiB Solid composition allocation; direct application-side Solid peak remains roughly 12 KiB plus saved registers and deeper firmware/helper calls. Intersection rebuilding has a 212-byte local allocation and writes directly into persistent cache storage.

This is a memory/stack **risk budget**, not a formal safety guarantee. EADK does not expose authoritative external-app RAM limits, firmware framebuffer ownership, firmware call depth, interrupt nesting, or remaining stack headroom. The linked static footprint is therefore known, but total runtime headroom cannot be proven from the relocatable ELF. Sustained four-function Ultra testing on physical hardware is mandatory before release.

## Rendering pipeline

Every graph redraw uses one immutable camera state. It projects the cached surface once, builds bounded coordinate geometry, waits for vertical blank, and produces the 27 graph bands from top to bottom. Each band uses this composition order:

1. Clear to the graph background.
2. Draw the world-space XY ground grid.
3. Draw world axes.
4. Draw coordinate-label background rectangles.
5. Draw numeric 5×7 bitmap labels.
6. Draw all wireframes, or all solids into one depth band.
7. Draw cached intersection geometry (depth-tested in Solid).
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
| `src/settings.rs` | Eight-row Settings interaction, Appearance/domain pages, optional performance readout, and the shared fixed 24-byte numeric editor |
| `src/expression.rs` | Streaming tokenizer, shunting-yard parser, fixed postfix bytecode, and stack evaluator |
| `src/function.rs` | `SurfaceFunction` boundary between evaluation and sampling |
| `src/functions.rs` | Four fixed function slots, independent drafts/bytecode/colors, and pair ordering/dirty helpers |
| `src/intersections.rs` | Six fixed marching-triangle caches, packed endpoints, overflow policy, and representative coordinates |
| `src/surface.rs` | Shared domain coordinates, four fixed height/light caches, resolution metadata, and discontinuity rejection |
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

Settings displays the manually maintained version string **`v3.0.0`**. Routine renderer, documentation, build, or packaging changes must not modify it; update it only when an explicit version change is requested. It is independent of automatic timestamps or generated artifacts.

## v3.0.0 changes

- Four independently editable and renderable mathematical surfaces.
- Shared fixed-capacity Ultra-resolution sampling and depth-tested multi-surface Solid rendering.
- Bounded numerical intersections for all six function pairs, with visibility controls and representative coordinates.
- Per-function palettes, Custom RGB colors, transactional expression editing, and preserved Wireframe/Solid controls.

## v2.7.0 changes

- Adds the fixed **Ultra** 41×31 sampling preset with 2,400 triangles alongside Low, Standard, and High.
- Extends the existing Appearance resolution selector without changing rendering, camera, lighting, color, depth, or Auto-Rotate behavior.
- Keeps maximum surface and projection capacity in persistent static storage so Ultra does not add resolution-sized stack arrays.
- Documents the measured 15,922-byte release-extreme `.bss` footprint and the public EADK ABI's lack of an authoritative external-app RAM/firmware-stack headroom figure.

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
