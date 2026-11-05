# Rift — compositor

**Rift is not part of Raze Linux 1.0.** No Rift code ships in this release, and the
installer does not offer it.

The default desktop is **niri** (upstream, unmodified), with the Raze session
around it (waybar, fuzzel, foot, mako, swaybg, swaylock, Raze configuration).
niri is a Rust, Smithay-based scrollable-tiling compositor; rewriting it would not
make anything faster by itself.

## What testing 1.0 taught us about Rift's purpose

niri refuses software EGL renderers. On machines and VMs without 3D acceleration it
shows a black screen. Raze 1.0 works around this with `raze-session`, which falls
back to sway using wlroots' pixman (CPU) renderer. A compositor that offers niri's
scrollable workflow **and** a first-class software renderer (Smithay ships a pixman
renderer) would remove the need for two compositors. That is the concrete
architectural difference Rift is meant to provide.

## Release criteria before Rift is offered

- DRM/KMS + libinput + libseat backend, with GLES and pixman renderers.
- Scrollable columns, workspaces, multi-monitor, configurable bindings.
- xdg-shell, layer-shell (waybar, fuzzel), xdg-activation, screencopy, XWayland.
- Runs the Raze session and Calamares; passes the desktop VM test without 3D.
- Measured idle CPU and memory compared with niri under the same workload.
