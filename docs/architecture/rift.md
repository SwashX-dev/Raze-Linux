# Rift — compositor

Rift is Raze Linux's own Wayland compositor and, since 1.1, the default desktop.
It is written in Rust on [Smithay](https://github.com/Smithay/smithay) (pinned to a
git revision in `rift/Cargo.toml`). It is not a wrapper around niri or Hyprland: the
layout engine, input handling, rendering, IPC and screencopy are Rift's own code in
`rift/src`.

## Why Rift exists

niri refuses software EGL renderers, so Raze 1.0 needed a second compositor (sway on
pixman) for machines without 3D acceleration. Rift offers the same scrollable-tiling
workflow with **two renderers**: GLES on GBM buffers when the GPU has hardware EGL,
and pixman on dumb buffers otherwise. One compositor covers every machine.

## Layout

Scrollable tiling, the model niri popularised:

- Each output has a list of **workspaces** (dynamic: there is always one empty
  workspace at the end; empty ones disappear when you leave them).
- A workspace is a horizontal strip of **columns** that can be wider than the
  screen; the view scrolls to keep the focused column visible (minimal scrolling,
  or centred with `layout.center_focused`). Opening a window never resizes others.
- A column holds one or more windows stacked vertically (`consume` / `expel`).
- Column widths are proportions of the working area (presets cycled with
  `switch-width`, ±10% steps, maximize, fullscreen).
- Scroll movement is animated (ease-out cubic, `animations.duration_ms`).

The layout engine (`layout.rs`) is pure: it computes placements from the tree and is
unit tested without a display. `state.rs` applies placements to Smithay's `Space`.
Every output is mapped in its own horizontal **lane** of the space, so a column
scrolled off one monitor never appears on its neighbour.

## Protocols

xdg-shell (+ decorations: server side, Rift draws a focus ring instead of title
bars), wlr-layer-shell, ext-session-lock, ext-idle-notify, idle-inhibit,
ext-foreign-toplevel-list, wlr-data-control, primary selection, data device (DnD),
xdg-activation, fractional-scale, viewporter, presentation-time, single-pixel-buffer,
cursor-shape, relative pointer, pointer constraints, linux-dmabuf (GLES), xdg-output,
**ext-image-copy-capture** and **wlr-screencopy** (screen capture into shm buffers,
used by grim and wf-recorder; `copy_with_damage` frames are only produced when the
screen changes). X11 applications run through xwayland-satellite.

## Backends

| Backend | When | Renderer |
|---|---|---|
| `udev` (DRM/KMS, libseat, libinput) | from a TTY / greetd | GLES (hardware EGL) or pixman (`RIFT_RENDERER=pixman`, or automatically when EGL is software-only) |
| `winit` | inside another Wayland/X11 session | GLES |
| `headless` | tests and CI | pixman, in memory |

Rendering is driven by damage: an output is redrawn only when something on it
changed, and frame callbacks are paced by vblank (or a refresh-rate timer when
nothing was drawn). At idle Rift does no work.

Rift links no systemd library: libinput is built against eudev and libseat against
elogind (`rift/package.nix`).

## Configuration

`~/.config/rift/config.toml`, falling back to `/etc/rift/config.toml` and the
built-in defaults (`rift/default-config.toml`, also the reference). User files are
merged over the defaults table by table, so they only need what they change; a bind
set to `"none"` is removed. Unknown keys are errors. An invalid file is reported
(`rift msg config-error`, `rift validate`) and the previous configuration is kept, so
a typo never leaves you without a desktop. Reload with Super+Shift+R.

Sections: `[input]` (keyboard layout/repeat, touchpad, focus follows mouse),
`[layout]` (gaps, width presets, focus ring colours), `[animations]`, `[cursor]`,
`[outputs."NAME"]` (scale, mode, position, disabled), `[[window_rules]]` (by app id
or title: width, workspace, maximized, fullscreen), `[binds]`, `[environment]`,
`spawn_at_startup`, `xwayland`.

## IPC

`$RIFT_SOCKET` (a Unix socket, one request per line, one JSON reply per line):

```bash
rift msg version          # {"version": "1.1.0", "backend": "drm-gles"}
rift msg workspaces       # per output, with window counts
rift msg windows
rift msg focused-window   # app id, title, output and geometry
rift msg action workspace 2
rift msg night-light 4000 # gamma ramp on every output; `off` restores it
rift msg subscribe        # stream the full state on every change (Rift Shell uses this)
```

Every `[binds]` action is also available through `rift msg action`.

## Tests

- `cargo test` in `rift/`: layout engine (scrolling, columns, workspaces, hotplug,
  rules, animation), configuration parsing/merging/validation, night-light ramps.
- The headless backend runs real clients in CI-like conditions: windows, layout
  actions over IPC, ext-image-copy-capture screenshots with grim, clipboard.
- The desktop VM test boots the r6 system and logs into Rift on DRM.

Benchmarks against niri: [performance/benchmarks.md](../performance/benchmarks.md).

## Limits in 1.1

- One GPU: Rift renders and scans out on the primary GPU only (no multi-GPU
  offload yet).
- Screencopy is shm-only (no dmabuf capture), so recorders that require dmabuf
  (wl-screenrec) do not work; screen recording
  uses wf-recorder with CPU encoding.
- No touch gestures and no tablet support yet; touchpad scrolling and taps work.
- No interactive mouse move/resize of tiled windows (keyboard driven, like niri's
  defaults).
