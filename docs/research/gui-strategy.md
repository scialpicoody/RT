# Research: minimal GUI strategy for `rt-gui`

- **Ticket:** [#8 (wayfinder:research)](https://github.com/scialpicoody/RT/issues/8)
- **Date:** 2026-11-02 (data gathered 2026-11-02)
- **Status:** Resolved — recommendation below feeds `rt-gui` implementation
- **Scope:** basic GUI only — one window, one widget, input events, simple 2D painting, on RT's single-threaded deterministic event loop. SOUP footprint and auditability are first-class criteria (medical-device trajectory).

## TL;DR recommendation

**Desktop dev loop: `winit 0.30` (window + input) + `softbuffer 0.4` (CPU pixel buffer → window) + `embedded-graphics 0.8` (paint model), with a ~50-line `DrawTarget` adapter we own.**
53 crates total (measured), all actively maintained under established orgs (rust-windowing, embedded-graphics), permissive licenses, zero GPU stack.

**Embedded: the same `embedded-graphics` paint code targets a real display driver (`mipidsi`, `ili9341`, `ssd1306`, …) — swap only the `DrawTarget` backend.** `embedded-graphics` is `no_std`, needs no allocator, and is the de-facto standard for small displays.

Rejected: egui (232-crate SOUP + immediate-mode clash with RT's retained object model), raqote (dormant), tiny-skia (excellent, but it is a rasterizer, not a pixel/window layer — keep as a possible optional AA-path engine later), framebuffer-only (cannot open a desktop window).

## Measurement method

Dependency counts below are **unique normal-dependency crates** measured today with `cargo tree -e normal --prefix none | sort -u` on scratch projects on macOS (host deps included; Linux adds/folds a few X11/Wayland crates). Raw lists in the research transcript; numbers are reproducible.

| Stack (as deps of a hello binary) | Unique crates | incl. self |
|---|---|---|
| `eframe 0.37` (egui) | 232 | huge |
| `raqote 0.8.5` default features | 53 | drags `font-kit` → core-text/core-graphics |
| `winit 0.30` + `softbuffer 0.4` + `embedded-graphics 0.8` | **53** | recommended |
| — of which `winit 0.30` alone | 36 | platform backends |
| — of which `softbuffer 0.4` + `embedded-graphics 0.8` | 23 | |
| `tiny-skia 0.12` default (`std`,`simd`,`png-format`) | 19 | PNG codec chain |
| `raqote 0.8.5` `default-features=false` | 11 | |
| `minifb 0.29` + `embedded-graphics 0.8` | 11 | honorable mention |

`embedded-graphics` itself contributes exactly **2 crates** (`embedded-graphics` + `embedded-graphics-core`) with **zero external dependencies** (verified in the `minifb` tree).

## Candidates

### 1. softbuffer + winit — RECOMMENDED foundation

- **What:** `winit` creates the window and delivers input events (the de-facto Rust windowing layer); `softbuffer` hands you a `&mut [u32]` CPU pixel buffer and presents it to that window. No GPU, no shaders, no render server protocol speak.
- **Pairing is blessed:** softbuffer's README documents the winit pairing verbatim ("first create a window using winit… create a Context and Surface…") and points at tiny-skia/vello_cpu for drawing primitives — i.e., drawing is deliberately out of scope, which is where embedded-graphics slots in. Sources: [softbuffer README](https://github.com/rust-windowing/softbuffer/blob/master/README.md), [softbuffer docs.rs](https://docs.rs/softbuffer).
- **Dependency footprint:** 53 crates combined (36 from winit — platform backends like X11/Wayland/Win32/macOS; 23 from softbuffer+e-g incl. overlap). The platform code has to live *somewhere*; winit is where the ecosystem has collectively audited it.
- **Maintenance:** both under the **rust-windowing** org. winit: 0.30.13 stable 2026-03-02, 0.31 in beta, last commit 2026-10-30 — very active. softbuffer: 0.4.8 released 2025-12-13, last commit 2026-07-18, 38 open issues — healthy. Sources: [crates.io API](https://crates.io/api/v1/crates/softbuffer), [winit repo](https://github.com/rust-windowing/winit), [softbuffer repo](https://github.com/rust-windowing/softbuffer).
- **License:** softbuffer `MIT OR Apache-2.0`; winit `Apache-2.0` (single, per crates.io for the 0.30.x line). Both permissive; Apache-2.0 adds an explicit patent grant. No copyleft, no commercial restrictions.
- **`no_std`/embedded:** not applicable — these are the *desktop* half of the story, replaced wholesale by a display driver on embedded.
- **Auditability:** safe public APIs; internally thin `unsafe` platform interop. `rt-gui` itself stays `#![forbid(unsafe_code)]`.
- **Effort to demo:** ~200–300 LOC (window, event loop hook, e-g DrawTarget adapter, paint a rect, click → signal). See architecture sketch.
- **Platform support:** Win32, macOS, X11, Wayland, and web (wasm) per softbuffer README.

### 2. embedded-graphics / framebuffer — RECOMMENDED paint model (both targets)

- **What:** immediate pixel-level drawing API (`DrawTarget` trait: pixels, lines, rects, circles, text via fonts) designed for small displays. We draw into *some* buffer; who owns the buffer is the backend.
- **Footprint:** 2 crates, zero external deps — the smallest possible SOUP add. Source: [embedded-graphics repo](https://github.com/embedded-graphics/embedded-graphics).
- **`no_std`:** core requirement of the project: "`no_std` compatible and works without a dynamic memory allocator" ([README](https://github.com/embedded-graphics/embedded-graphics#features)). Runs bare-metal on Cortex-M.
- **Maintenance:** slow-but-stable cadence — 0.8.0 (2023-05), 0.8.1 (2023-08), 0.8.2 (2026-02-15); last commit 2026-09-26; 90 open issues. The API has been stable for 3+ years; for a pixel-level library this is a feature, not rot. Source: [crates.io versions](https://crates.io/api/v1/crates/embedded-graphics/versions).
- **License:** `MIT OR Apache-2.0`.
- **Embedded story:** the de-facto standard — display drivers (`mipidsi`, `ili9341`, `ssd1306`, `sh1106`, `st7789`, …) implement `DrawTarget`, so the exact same widget/paint code runs against a SPI display on a microcontroller. Source: [embedded-graphics driver list](https://github.com/embedded-graphics/embedded-graphics/blob/master/README.md#drivers).
- **Desktop needs a presenter:** e-g cannot open a window — pair with softbuffer (desktop) or a display driver (embedded). On desktop we write the adapter ourselves (~50 LOC implementing `DrawTarget` over softbuffer's `&mut [u32]`): no adapter crate needed, and the adapter is the only format-sensitive code we own.
- **Caveat:** softbuffer's buffer pixel byte order is platform-dependent; the adapter must convert e-g's `Rgb888` to the target `u32` layout. Documented on softbuffer's docs.rs. Contained in those ~50 lines.

### 3. Small 2D crates: raqote, tiny-skia — NOT now (tiny-skia = later option)

- **raqote:** effectively dormant — last release 0.8.5 on **2024-09-11**, last commit **2025-02-11** (jrmuizel/raqote), single maintainer. Default features pull **`font-kit`** (→ core-text/core-graphics/core-foundation chains, dirs, walkdir — 53 crates) for text we don't need yet; `default-features=false` cuts it to 11 crates. License **BSD-3-Clause** (single). Sources: [repo](https://github.com/jrmuizel/raqote), [crates.io](https://crates.io/api/v1/crates/raqote). Verdict: maintenance risk too high for a SOUP-sensitive medical trajectory; drop.
- **tiny-skia:** strong — Skia subset, now under **Linebender** org, active (0.12.0 on 2026-02-02, last commit 2026-08-20). `#![no_std]`-capable (`no-std-float` feature; PNG optional), "fully safe" per its README, 19 crates default / fewer with `png-format` off. License **BSD-3-Clause**. Sources: [repo](https://github.com/linebender/tiny-skia), [lib.rs](https://github.com/linebender/tiny-skia/blob/main/src/lib.rs). Verdict: it solves *anti-aliased path rasterization*, which our basic scope (rects, lines, pixels, maybe bitmap text) doesn't need. e-g covers basic scope with 17 fewer crates. Keep as the designated upgrade if/when we need AA paths — it composes cleanly behind the same paint step.

### 4. egui (immediate-mode) — REJECTED

- **Footprint:** `eframe 0.37` = **232 unique crates** (wgpu/glow render stack, accesskit, image codecs, …). Measured today. This is the single largest SOUP item evaluated — an order of magnitude past everything else.
- **Architectural clash:** egui *is* a retained-state-immediate-mode UI framework with its own event model, layout, and repaint scheduling. RT already has an object model (metaobjects, typed signals/slots) and a deterministic event loop. Two competing event/object models in one process is exactly the coupling a small auditable runtime must avoid. We'd be auditing egui's model, not ours.
- **Maintenance/license:** very active (0.37.0 on 2026-10-15, emilk), `MIT OR Apache-2.0` — good, but activity doesn't shrink 232 crates. 1106 open issues indicates scale, not minimality. Source: [egui repo](https://github.com/emilk/egui), [crates.io](https://crates.io/api/v1/crates/egui).
- **`no_std`/embedded:** no — desktop/web oriented; no embedded story.
- Verdict: rejected for rt-gui. (Fine tool generally; wrong shape here.)

### Honorable mention: minifb

`minifb 0.29` + e-g = **11 crates** — one crate doing window + buffer present, no winit. Last release 2026-10-03, `MIT OR Apache-2.0`. Rejected as primary because: single-maintainer project vs rust-windowing org, weaker input-event taxonomy than winit (we need proper input routed into signals), and softbuffer/winit is the standard, better-governed path with comparable footprint after e-g is counted. Source: [rust_minifb](https://github.com/emoon/rust_minifb).

## Comparison matrix

| Criterion | softbuffer+winit+e-g | e-g/framebuffer only | raqote | tiny-skia | egui/eframe |
|---|---|---|---|---|---|
| Unique dep crates | **53** | 2 (+presenter required) | 53 (11 minimal) | 19 (fewer w/o png) | 232 |
| Maintenance | active orgs (rust-windowing, e-g) | e-g slow-stable | **dormant 20 mo** | active (Linebender) | very active |
| License | MIT/Ap-2.0 + Ap-2.0 | MIT OR Apache-2.0 | BSD-3-Clause | BSD-3-Clause | MIT OR Apache-2.0 |
| `no_std` / embedded | e-g: yes (no alloc) | **yes** | partially (raqote std-lean but untested claim) | yes (`no-std-float`) | no |
| Opens desktop window | **yes** | no | no | no | yes |
| Input events | **winit, complete** | no | no | no | yes (own model) |
| Fits RT object model | **yes — we own widgets** | n/a | n/a | n/a | **clashes** |
| Effort to basic demo | ~200–300 LOC | +presenter work | +winit anyway | +winit+softbuffer anyway | small but wrong shape |

## Recommended architecture (desktop dev loop)

```
winit event loop (main thread)
  WindowEvent::* ──translate──▶ rt-core typed Signals (mouse/key into object model)
  event-loop Flush/tick step
      widget tree paint pass ──▶ embedded-graphics DrawTarget
          SoftbufferTarget (ours, ~50 LOC): Rgb888 → u32, fills softbuffer &mut [u32]
              └─ softbuffer Surface::present()
```

- rt-gui stays `#![forbid(unsafe_code)]`; all `unsafe` lives in winit/softbuffer internals.
- winit 0.30's `ApplicationHandler` owns the thread's event loop; RT's deterministic loop is pumped from RedrawRequested/AboutToWait so single-threaded determinism is preserved. (If RT's loop must literally own the pump, winit's `EventLoopExtRunReturn`-style pumping on 0.30 is via `EventLoop::run` on a dedicated approach — validate in the demo; this is the one integration detail to prove early.)
- Pin `winit = "0.30"` (0.31 is beta; don't chase it).
- The `SoftbufferTarget` adapter is the only pixel-format-sensitive code we write; unit-test the RGB888→u32 conversion per platform.

## Embedded story (summary)

Same `embedded-graphics` paint code, different backend: on target hardware the `DrawTarget` is a display driver crate (`mipidsi`, `ili9341`, `ssd1306`, …) or a raw framebuffer — all `no_std`, no allocator. Desktop softbuffer backend and embedded driver backend are interchangeable behind our `DrawTarget`-shaped seam; widget code, signals, and the event-loop paint step are unchanged. If anti-aliased paths are ever needed, add tiny-skia (`no_std`-capable) behind the same seam as an optional feature.

## Risks & mitigations

1. **winit is 36 crates of platform code** — the largest chunk we accept. Mitigation: it's the ecosystem-standard, collectively audited home for that code; alternative (minifb) trades org governance for count. Accept.
2. **winit API churn across majors** (0.31 beta). Mitigation: pin 0.30.x; adapter surface is small (window creation + event translation).
3. **softbuffer pixel format is platform-dependent.** Mitigation: contained in one tested adapter.
4. **e-g release cadence is slow.** Mitigation: API stable 3+ years, zero-dep, pure Rust; worst case it's small enough to vendor/fork.
5. **Event-loop ownership detail** (winit run vs RT loop pump). Mitigation: first demo spike proves this integration before building widgets.

## Sources

- softbuffer: repo https://github.com/rust-windowing/softbuffer · README (winit pairing, platform list) · crates.io API (0.4.8, 2025-12-13, MIT OR Apache-2.0)
- winit: repo https://github.com/rust-windowing/winit (last commit 2026-10-30) · crates.io versions API (0.30.13 2026-03-02, Apache-2.0)
- embedded-graphics: repo https://github.com/embedded-graphics/embedded-graphics (no_std/no-alloc claim, driver list) · crates.io versions (0.8.2 2026-02-15)
- raqote: repo https://github.com/jrmuizel/raqote · Cargo.toml (font-kit optional under `text`) · crates.io (0.8.5 2024-09-11, BSD-3-Clause)
- tiny-skia: repo https://github.com/linebender/tiny-skia · lib.rs (`#![no_std]`) · README ("fully safe") · crates.io (0.12.0 2026-02-02, BSD-3-Clause)
- egui: repo https://github.com/emilk/egui · crates.io (0.37.0 2026-10-15)
- minifb: repo https://github.com/emoon/rust_minifb · crates.io (0.29.0 2026-10-03)
- Dependency counts: `cargo tree -e normal --prefix none | sort -u` in scratch projects, 2026-11-02
