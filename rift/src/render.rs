//! Building the render element list for an output.

use std::cell::RefCell;
use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::{AsRenderElements, Kind};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};
use smithay::desktop::layer_map_for_output;
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Scale, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::state::Rift;

smithay::backend::renderer::element::render_elements! {
    pub RiftElement<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<R>,
}

struct CursorImage {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Logical>,
}

type CursorCache = HashMap<(String, u32), Option<std::rc::Rc<CursorImage>>>;

/// XCursor theme images, loaded lazily per (name, size).
pub struct Cursors {
    theme: String,
    size: u32,
    cache: RefCell<CursorCache>,
}

impl Cursors {
    pub fn new(theme: &str, size: u32) -> Cursors {
        Cursors {
            theme: theme.to_string(),
            size,
            cache: RefCell::new(HashMap::new()),
        }
    }

    fn load(&self, name: &str, scale: u32) -> Option<std::rc::Rc<CursorImage>> {
        let size = self.size * scale.max(1);
        let key = (name.to_string(), size);
        if let Some(c) = self.cache.borrow().get(&key) {
            return c.clone();
        }
        let theme = xcursor::CursorTheme::load(&self.theme);
        let image = [name, "default", "left_ptr"]
            .iter()
            .find_map(|n| theme.load_icon(n))
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|data| xcursor::parser::parse_xcursor(&data))
            .and_then(|images| {
                images
                    .into_iter()
                    .min_by_key(|i| (i.size as i64 - size as i64).abs())
            })
            .map(|img| CursorImage {
                buffer: MemoryRenderBuffer::from_slice(
                    &img.pixels_rgba,
                    Fourcc::Abgr8888,
                    (img.width as i32, img.height as i32),
                    scale.max(1) as i32,
                    Transform::Normal,
                    None,
                ),
                hotspot: (
                    img.xhot as i32 / scale.max(1) as i32,
                    img.yhot as i32 / scale.max(1) as i32,
                )
                    .into(),
            })
            .or_else(|| Some(fallback_cursor()))
            .map(std::rc::Rc::new);
        self.cache.borrow_mut().insert(key, image.clone());
        image
    }
}

/// A plain arrow (white, black outline), used when no cursor theme is installed.
fn fallback_cursor() -> CursorImage {
    const W: usize = 13;
    const H: usize = 21;
    const ARROW: [(f32, f32); 7] = [
        (0.5, 0.5),
        (0.5, 17.0),
        (4.5, 13.0),
        (7.5, 20.0),
        (10.0, 19.0),
        (7.0, 12.0),
        (12.0, 12.0),
    ];
    // Even-odd point-in-polygon test at pixel centres.
    let inside = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
            return false;
        }
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut hit = false;
        for i in 0..ARROW.len() {
            let (ax, ay) = ARROW[i];
            let (bx, by) = ARROW[(i + 1) % ARROW.len()];
            if (ay > py) != (by > py) && px < ax + (py - ay) * (bx - ax) / (by - ay) {
                hit = !hit;
            }
        }
        hit
    };
    let mut px = vec![0u8; W * H * 4];
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if !inside(x, y) {
                continue;
            }
            let edge =
                !(inside(x - 1, y) && inside(x + 1, y) && inside(x, y - 1) && inside(x, y + 1));
            let c = if edge { 0 } else { 255 };
            let i = (y as usize * W + x as usize) * 4;
            px[i..i + 4].copy_from_slice(&[c, c, c, 255]);
        }
    }
    CursorImage {
        buffer: MemoryRenderBuffer::from_slice(
            &px,
            Fourcc::Abgr8888,
            (W as i32, H as i32),
            1,
            Transform::Normal,
            None,
        ),
        hotspot: (0, 0).into(),
    }
}

/// Solid buffers for focus rings, kept between frames so damage tracking works.
#[derive(Default)]
pub struct Decorations {
    rings: RefCell<HashMap<(WlSurface, u8), SolidColorBuffer>>,
}

fn color_of(s: &str, fallback: [f32; 4]) -> [f32; 4] {
    crate::config::parse_color(s).unwrap_or(fallback)
}

pub fn background_color(state: &Rift) -> [f32; 4] {
    if state.locked {
        return [0.0, 0.0, 0.0, 1.0];
    }
    color_of(&state.config.layout.background, [0.06, 0.06, 0.063, 1.0])
}

fn surface_elements<R>(
    renderer: &mut R,
    surface: &WlSurface,
    loc: Point<i32, Physical>,
    scale: Scale<f64>,
) -> Vec<RiftElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    render_elements_from_surface_tree(renderer, surface, loc, scale, 1.0, Kind::Unspecified)
}

/// All elements for `output`, front to back. Positions are output-local.
pub fn output_elements<R>(
    state: &Rift,
    renderer: &mut R,
    output: &Output,
    with_cursor: bool,
) -> Vec<RiftElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let mut out: Vec<RiftElement<R>> = Vec::new();
    let Some(ostate) = state.output_state(output) else {
        return out;
    };
    let lane = ostate.lane;
    let scale = Scale::from(output.current_scale().fractional_scale());

    if with_cursor
        && state.cursor_visible
        && state.output_at(state.pointer_location).as_ref() == Some(output)
    {
        out.extend(cursor_elements(state, renderer, output, lane, scale));
    }

    if state.locked {
        if let Some(lock) = &ostate.lock_surface {
            out.extend(surface_elements(
                renderer,
                lock.wl_surface(),
                (0, 0).into(),
                scale,
            ));
        }
        return out;
    }

    let map = layer_map_for_output(output);
    let layer_elems = |renderer: &mut R, layer: Layer| -> Vec<RiftElement<R>> {
        let mut v = Vec::new();
        for surface in map.layers_on(layer).rev() {
            let Some(geo) = map.layer_geometry(surface) else {
                continue;
            };
            v.extend(AsRenderElements::<R>::render_elements::<RiftElement<R>>(
                surface,
                renderer,
                geo.loc.to_physical_precise_round(scale),
                scale,
                1.0,
            ));
        }
        v
    };

    let fullscreen = state
        .focused_rect_on(output)
        .map(|(_, full)| full)
        .unwrap_or(false);

    out.extend(layer_elems(renderer, Layer::Overlay));
    if !fullscreen {
        out.extend(layer_elems(renderer, Layer::Top));
    }

    // Windows (with their popups) and focus rings.
    let ring_w = state.config.layout.focus_ring_width.max(0);
    let active = color_of(&state.config.layout.active_color, [0.86, 0.15, 0.15, 1.0]);
    let inactive = color_of(&state.config.layout.inactive_color, [0.25, 0.25, 0.27, 1.0]);
    let focused = state.focused_window().cloned();
    let size = state.output_logical_size(output);
    let mut rings = state.decorations.rings.borrow_mut();
    let mut used = Vec::new();
    for window in state.space.elements_for_output(output).rev() {
        let Some(loc) = state.space.element_location(window) else {
            continue;
        };
        let geo = window.geometry();
        let local = Point::from((loc.x - lane, loc.y));
        if local.x >= size.w || local.x + geo.size.w <= 0 {
            continue;
        }
        let render_loc = (local - geo.loc).to_physical_precise_round(scale);
        out.extend(AsRenderElements::<R>::render_elements::<RiftElement<R>>(
            window, renderer, render_loc, scale, 1.0,
        ));

        let is_focused = Some(window) == focused.as_ref();
        let full = state.layout.is_fullscreen(window);
        if ring_w > 0 && !full {
            let Some(surface) = window.toplevel().map(|t| t.wl_surface().clone()) else {
                continue;
            };
            let color = if is_focused { active } else { inactive };
            let (x, y, w, h, b) = (local.x, local.y, geo.size.w, geo.size.h, ring_w);
            let sides = [
                (x - b, y - b, w + 2 * b, b),
                (x - b, y + h, w + 2 * b, b),
                (x - b, y, b, h),
                (x + w, y, b, h),
            ];
            for (i, (sx, sy, sw, sh)) in sides.into_iter().enumerate() {
                let buf = rings.entry((surface.clone(), i as u8)).or_default();
                buf.update((sw, sh), color);
                used.push((surface.clone(), i as u8));
                out.push(RiftElement::Solid(SolidColorRenderElement::from_buffer(
                    buf,
                    Point::<i32, Logical>::from((sx, sy)).to_physical_precise_round(scale),
                    scale,
                    1.0,
                    Kind::Unspecified,
                )));
            }
        }
    }
    rings.retain(|k, _| used.contains(k));
    drop(rings);

    out.extend(layer_elems(renderer, Layer::Bottom));
    out.extend(layer_elems(renderer, Layer::Background));
    out
}

fn cursor_elements<R>(
    state: &Rift,
    renderer: &mut R,
    output: &Output,
    lane: i32,
    scale: Scale<f64>,
) -> Vec<RiftElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let pos = state.pointer_location - Point::from((lane as f64, 0.0));
    let mut out = Vec::new();
    if let Some(icon) = &state.dnd_icon {
        out.extend(surface_elements(
            renderer,
            icon,
            pos.to_physical_precise_round(scale),
            scale,
        ));
    }
    match &state.cursor_status {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .map(|d| d.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            let loc = (pos - hotspot.to_f64()).to_physical_precise_round(scale);
            out.extend(surface_elements(renderer, surface, loc, scale));
        }
        CursorImageStatus::Named(icon) => {
            let int_scale = output.current_scale().integer_scale().max(1) as u32;
            if let Some(img) = state.cursors.load(icon.name(), int_scale) {
                let loc: Point<f64, Physical> = (pos - img.hotspot.to_f64()).to_physical(scale);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    loc,
                    &img.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(e) => out.push(RiftElement::Memory(e)),
                    Err(e) => tracing::warn!("cannot upload cursor: {e:?}"),
                }
            }
        }
    }
    out
}
