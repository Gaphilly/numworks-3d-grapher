# NumWorks 3D Grapher

NumWorks 3D Grapher is a native Rust `no_std` application for plotting mathematical surfaces on a NumWorks calculator through the EADK. It compiles `z = f(x, y)` expressions into compact postfix bytecode, samples them on a fixed grid, and renders an interactive perspective wireframe without a heap, GPU, filesystem, framebuffer, or depth buffer.

The project targets `thumbv7em-none-eabihf` (the calculator's Cortex-M7-class Thumb-2 CPU with a hardware single-precision floating-point ABI) and is based on NumWorks' official `epsilon-sample-app-rust` template.

## Current features

- Interactive 3D wireframe plotting on a configurable rectangular XY domain
- `f32` expression compiler/evaluator supporting constants, `x`, `y`, arithmetic, power, parentheses, unary minus, and `sin`, `cos`, `tan`, `sqrt`, and `abs`
- Calculator-style Equation editor with a 96-byte buffer, cursor, scrolling, shortcuts, errors, and held-key repeat
- Graph, Equation, and Settings tabs with explicit focus and dirty-region rendering
- World-space X/Y/Z axes, origin, sparse 1/2/5 grid and ticks, and stable 5×7 coordinate labels
- Orbit, truck/track, pedestal, true dolly, and independent perspective/FOV controls
- Fixed-capacity, no-heap architecture designed for the calculator's limited RAM and ARM CPU
- Host-side tests for parsing, editing, camera math, sampling, projection, ticks, and labels

Filled surfaces, lighting, persistent settings, and equation/domain settings UI are intentionally not implemented yet.

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

Camera keys use raw keyboard state and repeat smoothly while held. Distance, pitch, focal length, and translations are clamped to finite practical ranges.

### Tab bar

| Input | Action |
| --- | --- |
| Left / Right | Highlight the previous / next tab |
| OK | Activate the highlighted tab and focus its content |
| Back | Cancel tab navigation and return to current content |

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
        ↓
clipped projected surface/grid/axes/labels
        ↓
320×8 RGB565 band rasterizer
        ↓
27 EADK display transfers
```

The input/UI path is deliberately split:

```text
eadk_keyboard_scan() ──→ raw edges ──→ tab/focus state machine
          │
          └────────────→ held state ─→ continuous camera + editor repeat

bounded eadk_event_get() ────────────→ semantic Equation characters

AppState dirty flags ────────────────→ header / content / surface redraws
```

Raw state owns continuous motion and application-level focus. Semantic events are polled only after a raw down edge in tab/editor contexts; there is no blocking editor loop. Camera changes invalidate projection/content but reuse cached heights. Only a successful equation compile (or a future domain change) resamples the surface.

## Memory model

There is no allocator and no heap-backed collection. Important fixed storage includes:

| Storage | Size / capacity | Lifetime |
| --- | ---: | --- |
| RGB565 band | 320×8×2 = 5,120 bytes | One render pass |
| Cached surface heights | 25×19 `f32` = 1,900 bytes, plus range metadata | Active graph |
| Projected surface cache | 25×19 `(i16, i16)` = 1,900 bytes | One render pass |
| Expression source | 96 bytes | Editor state |
| Postfix bytecode | 64 fixed instructions | Active expression |
| Evaluation stack | 32×`f32` = 128 bytes | One sample evaluation |
| Parser operator stack | 32 fixed operators | One compilation |
| Coordinate geometry/labels | 48 lines, 12 labels, at most 12 ticks per axis | One render pass |
| Bitmap label glyphs | 5×7 static flash data for digits/signs and X/Y/Z | Program image |

A full 320×240 RGB565 framebuffer would consume 153,600 bytes, so the graph uses the 5,120-byte band instead. A full-screen depth buffer would add at least another 76,800 bytes even at 16 bits and is also omitted. Wireframe depth is handled by composition and conservative label placement rather than per-pixel depth testing. Drawing occurs into RAM bands and uses one EADK rectangle transfer per band—never one firmware call per pixel.

## Rendering pipeline

Every graph redraw uses one immutable camera state. It projects the cached surface once, builds bounded coordinate geometry, waits for vertical blank, and produces the 27 graph bands from top to bottom. Each band uses this exact composition order:

1. Clear to graph background.
2. Draw XY grid.
3. Draw world axes.
4. Draw coordinate-label background rectangles.
5. Draw numeric 5×7 bitmap labels.
6. Draw the surface wireframe.
7. Draw tick marks and origin.
8. Draw X/Y/Z bitmap labels.
9. Push the completed band with EADK.

Graph labels are part of the band buffer. Do not replace them with `eadk_display_draw_string()` or draw directly to the display between band transfers: doing so can expose stale labels during camera redraws and was the cause of a previous flashing/alternating-frame artifact. Firmware text drawing remains appropriate for independently redrawn header, Equation, and Settings UI regions.

There is no depth buffer. Near-plane line clipping and non-finite checks keep unsafe projected values away from the rasterizer; label selection is intentionally conservative rather than pretending to offer perfect text/surface occlusion.

## Project structure

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | NWA metadata, exported entry point, cooperative input/render loop, and dirty-surface cache ownership |
| `src/app.rs` | Tab/content focus state machine and header/content/surface dirty flags |
| `src/eadk.rs` | Rust FFI layout, constants, and guarded wrappers for firmware display, keyboard, event, and timing symbols |
| `src/editor.rs` | Fixed 96-byte Equation editor, cursor/scroll logic, calculator event mapping, and held-key repeat |
| `src/expression.rs` | Streaming tokenizer, shunting-yard parser, fixed postfix bytecode, and stack evaluator |
| `src/function.rs` | `SurfaceFunction` boundary between evaluation and sampling |
| `src/surface.rs` | Domain mapping, 25×19 sampling, cached heights, and world points |
| `src/camera.rs` | Orbit target/distance state, translation, perspective projection, and near-plane line clipping |
| `src/graph.rs` | Central RGB565 palette, domain-aware axis visibility, and bounded 1/2/5 ticks |
| `src/rendering.rs` | Projected caches, coordinate/label geometry, 5×7 glyphs, band rasterization, and EADK transfers |
| `src/input.rs` | Smooth raw-key graph camera mapping |
| `src/ui.rs` | Tab header, Equation content, Settings placeholder, and user-facing parse errors |
| `src/math.rs` | Small `no_std` `f32` trigonometric, root, and power approximations |
| `src/icon.png` | Application icon source converted to NWI during embedded builds |
| `build.rs` | Host-side timestamped PNG-to-NWI conversion through nwlink |
| `.cargo/config.toml` | Default ARM target, relocatable linker settings, and pinned install runner |

## Development

Prerequisites are Rust/rustup, Node.js/npm, nwlink-compatible USB access, and a supported NumWorks calculator. Install the embedded target once:

```bash
rustup target add thumbv7em-none-eabihf
```

Build the embedded relocatable application:

```bash
cargo build
```

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

Useful pre-handoff checks are:

```bash
cargo fmt -- --check
cargo test --target x86_64-unknown-linux-gnu
cargo build
git diff --check
```

Rendering, keyboard, event, and timing behavior must also be verified on real hardware; host tests cannot reproduce display transfer timing or the physical key/event interaction.

## Contributing and development rules

- Keep target code `no_std` and allocation-free unless a deliberate, measured design change justifies otherwise.
- Respect fixed capacities, validate indices/pointer lengths before FFI, and preserve EADK C ABI layouts.
- Prefer `f32`; reject NaN/infinity before converting to screen coordinates.
- Do not add a full-screen framebuffer/depth buffer or per-pixel EADK calls.
- Preserve the graph band composition order and raw/semantic input ownership split.
- Add host tests for non-trivial parser, editor, geometry, camera, or numerical logic.
- Test rendering/input changes on a physical calculator.
- For every significant subsystem change, document its public API, invariants, memory/performance impact, and user-visible controls here.

## License and trademarks

This project retains the template's BSD license; see [LICENSE](LICENSE). NumWorks and Rust are trademarks of their respective owners.
