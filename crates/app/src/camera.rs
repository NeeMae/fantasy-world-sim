//! Pan and zoom.
//!
//! Zoom eases towards a target level rather than jumping per wheel notch,
//! and keeps the point under the cursor fixed. Dragging "grabs" the map, so
//! the point you grabbed stays under the cursor regardless of mouse
//! acceleration or display scaling.

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;

use crate::map_view::MapView;
use crate::tools::{self, PointerOverUi};

const MIN_SCALE: f32 = 0.125;
const MAX_SCALE: f32 = 8.0;
/// Fraction of the remaining zoom covered per second is `1 - e^-ZOOM_RATE`.
const ZOOM_RATE: f32 = 18.0;

#[derive(Component)]
pub struct MapCamera {
    target_scale: f32,
    /// World point held under the cursor while dragging.
    grab: Option<Vec2>,
}

/// Room the UI panels take from the window (right, top), in logical pixels.
/// Used only to frame the map nicely; the panels may be resized later.
const PANEL_SPACE: Vec2 = Vec2::new(260.0, 24.0);

/// Spawns the camera framing the whole map in the part of the window not
/// covered by panels.
pub fn spawn(commands: &mut Commands, map_size: Vec2, window_size: Vec2) {
    let (scale, translation) = framing(map_size, window_size);
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection { scale, ..OrthographicProjection::default_2d() }),
        Transform::from_translation(translation),
        MapCamera { target_scale: scale, grab: None },
    ));
}

/// Recentres and zooms to fit a newly generated map.
pub fn refit(
    camera: &mut Query<(&Camera, &mut Transform, &mut Projection, &mut MapCamera)>,
    map_size: Vec2,
    window_size: Vec2,
) {
    let Ok((_, mut transform, mut projection, mut cam)) = camera.single_mut() else { return };
    let (scale, translation) = framing(map_size, window_size);
    if let Projection::Orthographic(ortho) = projection.as_mut() {
        ortho.scale = scale;
    }
    cam.target_scale = scale;
    transform.translation = translation;
}

/// Zoom and camera position that fit the map in the uncovered window area.
fn framing(map_size: Vec2, window_size: Vec2) -> (f32, Vec3) {
    let visible = (window_size - PANEL_SPACE).max(Vec2::splat(100.0));
    let scale = ((map_size / visible).max_element() * 1.04).clamp(MIN_SCALE, MAX_SCALE);
    // Shift so the map's centre sits in the middle of the uncovered area.
    let offset = Vec2::new(PANEL_SPACE.x / 2.0, PANEL_SPACE.y / 2.0) * scale;
    (scale, offset.extend(0.0))
}

/// WASD/arrows or right/middle-drag to pan, mouse wheel to zoom towards the cursor.
pub fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    over_ui: Res<PointerOverUi>,
    time: Res<Time>,
    windows: Query<&Window>,
    view: Res<MapView>,
    mut camera: Query<(&Camera, &mut Transform, &mut Projection, &mut MapCamera)>,
) {
    let Ok((camera, mut transform, mut projection, mut cam)) = camera.single_mut() else { return };
    let Projection::Orthographic(ortho) = projection.as_mut() else { return };
    let cursor = tools::cursor(&windows);
    let over_ui = over_ui.blocks(cursor);
    // Use this frame's transform rather than last frame's GlobalTransform so
    // earlier changes this frame are accounted for.
    let to_world = |transform: &Transform, screen: Vec2| {
        camera.viewport_to_world_2d(&GlobalTransform::from(*transform), screen).ok()
    };

    // Keyboard pan.
    if !egui.wants_any_keyboard_input() {
        let mut dir = Vec2::ZERO;
        for (keys_for, d) in [
            ([KeyCode::KeyW, KeyCode::ArrowUp], Vec2::Y),
            ([KeyCode::KeyS, KeyCode::ArrowDown], Vec2::NEG_Y),
            ([KeyCode::KeyA, KeyCode::ArrowLeft], Vec2::NEG_X),
            ([KeyCode::KeyD, KeyCode::ArrowRight], Vec2::X),
        ] {
            if keys.any_pressed(keys_for) {
                dir += d;
            }
        }
        transform.translation +=
            (dir.normalize_or_zero() * 600.0 * ortho.scale * time.delta_secs()).extend(0.0);
    }

    // Grab-drag pan.
    let drag_buttons = [MouseButton::Right, MouseButton::Middle];
    if buttons.any_just_pressed(drag_buttons) && !over_ui {
        cam.grab = cursor.and_then(|c| to_world(&transform, c));
    }
    if !buttons.any_pressed(drag_buttons) {
        cam.grab = None;
    }
    if let (Some(grab), Some(c)) = (cam.grab, cursor)
        && let Some(now) = to_world(&transform, c)
    {
        transform.translation += (grab - now).extend(0.0);
    }

    // Zoom: wheel input sets a target; the scale eases towards it.
    if !over_ui {
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 60.0,
        };
        if notches != 0.0 {
            cam.target_scale = (cam.target_scale * 0.85f32.powf(notches)).clamp(MIN_SCALE, MAX_SCALE);
        }
    }
    let old = ortho.scale;
    if (old - cam.target_scale).abs() > f32::EPSILON {
        let t = 1.0 - (-ZOOM_RATE * time.delta_secs()).exp();
        // Interpolate in log space so zooming in and out feel symmetric.
        let mut new = old * (cam.target_scale / old).powf(t);
        if (new / cam.target_scale - 1.0).abs() < 0.001 {
            new = cam.target_scale;
        }
        // Keep the point under the cursor fixed (unless dragging, where the
        // grab already pins a point).
        if cam.grab.is_none()
            && let Some(p) = cursor.and_then(|c| to_world(&transform, c))
        {
            let camera_pos = transform.translation.truncate();
            transform.translation = (p + (camera_pos - p) * (new / old)).extend(transform.translation.z);
        }
        ortho.scale = new;
    }

    // Don't let the map be lost off-screen.
    let half = view.size / 2.0;
    transform.translation.x = transform.translation.x.clamp(-half.x, half.x);
    transform.translation.y = transform.translation.y.clamp(-half.y, half.y);
}
