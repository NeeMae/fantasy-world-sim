//! Draws the world as one pixel-art texture and maps the cursor back to hexes.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use content::Registry;
use map_raster::RenderOptions;
use sim_core::{HexId, World};

use crate::sim_thread::SimThread;

#[derive(Resource)]
pub struct MapView {
    pub registry: Arc<Registry>,
    pub options: RenderOptions,
    image: Handle<Image>,
    drawn_revision: Option<u64>,
    /// Image size in pixels; the sprite is centred on the origin.
    pub size: Vec2,
}

impl MapView {
    /// Converts a world-space position to the hex under it.
    pub fn hex_at(&self, world: &World, pos: Vec2) -> Option<HexId> {
        let px = pos.x + self.size.x / 2.0;
        let py = self.size.y / 2.0 - pos.y;
        self.options.layout.hex_at(&world.topology, px, py)
    }

    /// World-space centre of a hex.
    pub fn hex_center(&self, world: &World, hex: HexId) -> Vec2 {
        let (x, y) = self.options.layout.center(&world.topology, hex);
        Vec2::new(x - self.size.x / 2.0, self.size.y / 2.0 - y)
    }
}

#[derive(Resource, Default)]
pub struct Selection(pub Option<HexId>);

#[derive(Component)]
pub struct SelectionMarker;

/// Spawns the map sprite and selection marker; returns the map's size in pixels.
pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    registry: Arc<Registry>,
    world: &World,
) -> Vec2 {
    let options = RenderOptions::default();
    let (w, h) = options.layout.image_size(&world.topology);
    let image = images.add(blank_image(w, h));
    commands.spawn((Sprite::from_image(image.clone()), Transform::default()));
    commands.spawn((
        Sprite::from_color(Color::srgba(1.0, 0.9, 0.3, 0.55), Vec2::splat(options.layout.size * 1.6)),
        Transform::from_xyz(0.0, 0.0, 1.0),
        Visibility::Hidden,
        SelectionMarker,
    ));
    let size = Vec2::new(w as f32, h as f32);
    commands.insert_resource(MapView { registry, options, image, drawn_revision: None, size });
    size
}

fn blank_image(width: u32, height: u32) -> Image {
    Image::new_fill(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

/// Re-rasterises the map whenever the simulation reports changed terrain.
pub fn redraw(sim: Res<SimThread>, mut view: ResMut<MapView>, mut images: ResMut<Assets<Image>>) {
    let snap = sim.snapshot();
    if view.drawn_revision == Some(snap.world.terrain_revision) {
        return;
    }
    let raster = map_raster::render(&snap.world, &view.registry, &view.options);
    if let Some(mut image) = images.get_mut(&view.image) {
        image.data = Some(raster.rgba);
    }
    view.drawn_revision = Some(snap.world.terrain_revision);
}

/// Keeps the selection marker on the selected hex, at a constant on-screen size.
pub fn update_marker(
    selection: Res<Selection>,
    sim: Res<SimThread>,
    view: Res<MapView>,
    camera: Query<&Projection, With<Camera>>,
    mut marker: Query<(&mut Transform, &mut Visibility), With<SelectionMarker>>,
) {
    let Ok((mut transform, mut visibility)) = marker.single_mut() else { return };
    match selection.0 {
        Some(hex) => {
            let c = view.hex_center(&sim.snapshot().world, hex);
            transform.translation = c.extend(1.0);
            if let Ok(Projection::Orthographic(ortho)) = camera.single() {
                transform.scale = Vec3::splat(ortho.scale.max(1.0));
            }
            *visibility = Visibility::Visible;
        }
        None => *visibility = Visibility::Hidden,
    }
}
