//! Draws the world as a grid of pixel-art texture chunks, redraws only the
//! chunks whose hexes change, and overlays hex outlines for hover/selection.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use content::Registry;
use map_raster::{PixelRect, RenderOptions};
use sim_core::{HexId, World};

use crate::sim_thread::SimThread;
use crate::tools::{Hover, Selection, ToolState};

/// Chunk edge in pixels. Small enough that redrawing one is cheap and that
/// huge maps stay under GPU texture size limits.
const CHUNK: u32 = 512;

/// Everything belonging to the current world, despawned on regenerate.
#[derive(Component)]
pub struct MapEntity;

struct Chunk {
    rect: PixelRect,
    image: Handle<Image>,
}

#[derive(Resource)]
pub struct MapView {
    pub registry: Arc<Registry>,
    pub options: RenderOptions,
    chunks: Vec<Chunk>,
    /// Image size in pixels; the map is centred on the world origin.
    pub size: Vec2,
    /// East-west wrapping: the map is drawn three times side by side and the
    /// camera is kept over the middle copy, so panning never reaches an edge.
    pub wrap: bool,
}

impl MapView {
    /// Horizontal offsets, in map widths, of each drawn copy of the map.
    fn copies(&self) -> &'static [f32] {
        if self.wrap { &[-1.0, 0.0, 1.0] } else { &[0.0] }
    }

    /// The hex under a world-space position.
    pub fn hex_at(&self, world: &World, pos: Vec2) -> Option<HexId> {
        let (px, py) = self.to_pixels(pos);
        self.options.layout.hex_at(&world.topology, px, py)
    }

    fn to_pixels(&self, pos: Vec2) -> (f32, f32) {
        (pos.x + self.size.x / 2.0, self.size.y / 2.0 - pos.y)
    }

    /// World-space centre of a pixel rectangle.
    fn rect_center(&self, r: PixelRect) -> Vec2 {
        Vec2::new(
            r.x as f32 + r.width as f32 / 2.0 - self.size.x / 2.0,
            self.size.y / 2.0 - (r.y as f32 + r.height as f32 / 2.0),
        )
    }

    fn redraw_chunk(&self, chunk: &Chunk, world: &World, images: &mut Assets<Image>) {
        let rgba = map_raster::render_rect(world, &self.registry, &self.options, chunk.rect);
        if let Some(mut image) = images.get_mut(&chunk.image) {
            image.data = Some(rgba);
        }
    }
}

/// Spawns the map for `world`; returns its size in pixels.
pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    registry: Arc<Registry>,
    world: &World,
) -> Vec2 {
    let options = RenderOptions::default();
    let (w, h) = options.layout.image_size(&world.topology);
    let size = Vec2::new(w as f32, h as f32);
    let wrap = world.topology.wrap() == sim_core::Wrap::X;
    let mut view = MapView { registry, options, chunks: Vec::new(), size, wrap };

    for cy in (0..h).step_by(CHUNK as usize) {
        for cx in (0..w).step_by(CHUNK as usize) {
            let rect =
                PixelRect { x: cx as i32, y: cy as i32, width: CHUNK.min(w - cx), height: CHUNK.min(h - cy) };
            let rgba = map_raster::render_rect(world, &view.registry, &view.options, rect);
            let image = images.add(make_image(rect.width, rect.height, rgba));
            for &copy in view.copies() {
                let pos = view.rect_center(rect) + Vec2::X * copy * size.x;
                commands.spawn((
                    Sprite::from_image(image.clone()),
                    Transform::from_translation(pos.extend(0.0)),
                    MapEntity,
                ));
            }
            view.chunks.push(Chunk { rect, image });
        }
    }

    for kind in [OutlineKind::Hover, OutlineKind::Selection] {
        for &copy in view.copies() {
            commands.spawn((
                Sprite::default(),
                Transform::from_xyz(0.0, 0.0, kind.z()),
                Visibility::Hidden,
                Outline { kind, copy, key: None },
                MapEntity,
            ));
        }
    }
    commands.insert_resource(view);
    size
}

fn make_image(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    Image::new(
        Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        TextureDimension::D2,
        if rgba.is_empty() { vec![0; 4] } else { rgba },
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

/// Redraws the chunks covering hexes the simulation changed.
pub fn redraw_changes(sim: Res<SimThread>, view: Res<MapView>, mut images: ResMut<Assets<Image>>) {
    let changed = sim.take_changed_hexes();
    if changed.is_empty() {
        return;
    }
    let world = sim.snapshot().world;
    let Some(dirty) = map_raster::hexes_bounds(&world.topology, &view.options.layout, &changed) else {
        return;
    };
    for chunk in &view.chunks {
        if overlaps(chunk.rect, dirty) {
            view.redraw_chunk(chunk, &world, &mut images);
        }
    }
}

fn overlaps(a: PixelRect, b: PixelRect) -> bool {
    a.x < b.x + b.width as i32
        && b.x < a.x + a.width as i32
        && a.y < b.y + b.height as i32
        && b.y < a.y + a.height as i32
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OutlineKind {
    Hover,
    Selection,
}

impl OutlineKind {
    fn z(self) -> f32 {
        match self {
            OutlineKind::Hover => 1.0,
            OutlineKind::Selection => 2.0,
        }
    }

    fn color(self) -> [u8; 4] {
        match self {
            OutlineKind::Hover => [255, 255, 255, 200],
            OutlineKind::Selection => [255, 214, 64, 255],
        }
    }
}

/// An outline overlay, rebuilt only when what it traces changes.
#[derive(Component)]
pub struct Outline {
    kind: OutlineKind,
    /// Which copy of a wrapping map this is drawn over, in map widths.
    copy: f32,
    /// The hexes and line thickness currently drawn.
    key: Option<(Vec<HexId>, u32)>,
}

/// Keeps the hover and selection outlines tracing the right hexes, with a
/// line thick enough to see at the current zoom.
pub fn update_outlines(
    hover: Res<Hover>,
    selection: Res<Selection>,
    tools: Res<ToolState>,
    sim: Res<SimThread>,
    view: Res<MapView>,
    camera: Query<&Projection, With<Camera>>,
    mut images: ResMut<Assets<Image>>,
    mut outlines: Query<(&mut Outline, &mut Sprite, &mut Transform, &mut Visibility)>,
) {
    let zoom = match camera.single() {
        Ok(Projection::Orthographic(o)) => o.scale,
        _ => 1.0,
    };
    let thickness = (zoom.ceil() as u32).clamp(1, 4);
    let world = sim.snapshot().world;
    let mut built: Vec<(OutlineKind, Handle<Image>, PixelRect)> = Vec::new();

    for (mut outline, mut sprite, mut transform, mut visibility) in &mut outlines {
        let hexes = match outline.kind {
            OutlineKind::Hover => hover.0.map(|h| tools.affected_hexes(&world, h)).unwrap_or_default(),
            OutlineKind::Selection => selection.0.into_iter().collect(),
        };
        if hexes.is_empty() {
            *visibility = Visibility::Hidden;
            outline.key = None;
            continue;
        }
        let key = (hexes, thickness);
        if outline.key.as_ref() == Some(&key) {
            continue;
        }
        // Copies over a wrapping map share one traced image.
        let (image, rect) = match built.iter().find(|(kind, ..)| *kind == outline.kind) {
            Some((_, image, rect)) => (image.clone(), *rect),
            None => {
                let Some(drawn) = map_raster::outline(
                    &world.topology,
                    &view.options.layout,
                    &key.0,
                    thickness,
                    outline.kind.color(),
                ) else {
                    continue;
                };
                let image = images.add(make_image(drawn.rect.width, drawn.rect.height, drawn.rgba));
                built.push((outline.kind, image.clone(), drawn.rect));
                (image, drawn.rect)
            }
        };
        // Replacing the handle frees the previous outline image.
        sprite.image = image;
        let pos = view.rect_center(rect) + Vec2::X * outline.copy * view.size.x;
        transform.translation = pos.extend(outline.kind.z());
        *visibility = Visibility::Visible;
        outline.key = Some(key);
    }
}
