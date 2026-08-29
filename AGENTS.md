# NumWorks 3D Grapher — Agent Instructions

## Project

This is a native Rust application for the NumWorks graphing calculator using the NumWorks EADK.

The application targets:

`thumbv7em-none-eabihf`

The application is `#![no_std]` and `#![no_main]`.

The project must remain compatible with the NumWorks EADK and must run on real NumWorks hardware.

## Build

The normal development commands are:

```bash
cargo build
cargo run
```

`cargo run` installs the application onto a connected NumWorks calculator through nwlink.

The project currently uses nwlink 0.0.19.

Do not change the nwlink version or target configuration unless explicitly requested.

## Platform constraints

Assume extremely constrained embedded hardware.

* 320×240 display
* RGB565 display format
* Limited RAM
* No GPU
* No operating system
* No filesystem
* No desktop APIs
* No standard library
* No heap allocation unless explicitly introduced and justified
* Prefer `f32` over `f64`
* Avoid per-pixel EADK calls
* Avoid full-screen framebuffer allocations
* Avoid unnecessary copies

Prefer fixed-capacity data structures and stack/static allocation.

## Safety

Never assume an EADK function is safe merely because its Rust wrapper looks safe.

Validate buffer sizes and array bounds before FFI calls.

Avoid `unwrap()`, unchecked indexing, and arithmetic that can panic.

Remember that the panic handler does not provide useful recovery.

## Rendering

The initial renderer should be wireframe.

The first milestone is a hard-coded mathematical surface such as:

`z = sin(x) * cos(y)`

The first implementation should prioritize:

1. Correctness
2. Real hardware performance
3. Low memory usage
4. Simple architecture

Do not implement a full 3D engine prematurely.

Do not introduce external crates unless there is a clear reason and the crate is compatible with `no_std` and the target.

## Development methodology

Before making large architectural changes:

* Inspect the existing code.
* Make the smallest change that demonstrates the next capability.
* Build with `cargo build`.
* Test on real hardware with `cargo run`.
* Keep commits small and logically separated.

Do not rewrite working infrastructure without a concrete reason.

## Current goal

Build a responsive 3D mathematical surface grapher capable of displaying:

`z = f(x, y)`

with:

* camera rotation
* zoom
* mathematical expression evaluation
* wireframe rendering
* calculator-key controls

Start with a hard-coded function before implementing an expression parser.
