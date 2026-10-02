//! Pan and zoom.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;

use crate::map_view::MapView;

const MIN_SCALE: f32 = 0.125;
const MAX_SCALE: f32 = 8.0;

/// Spawns the camera zoomed out so a `map_size` map fits in a `window_size` window.
pub fn spawn(commands: &mut Commands, map_size: Vec2, window_size: Vec2) {
    let fit = (map_size / window_size.max(Vec2::ONE)).max_element();
    let scale = (fit * 1.05).clamp(MIN_SCALE, MAX_SCALE);
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection { scale, ..OrthographicProjection::default_2d() }),
    ));
}

/// WASD/arrows or right/middle-drag to pan, mouse wheel to zoom towards the cursor.
#[allow(clippy::too_many_arguments)]
pub fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    time: Res<Time>,
    windows: Query<&Window>,
    view: Res<MapView>,
    mut camera: Query<(&Camera, &GlobalTransform, &mut Transform, &mut Projection)>,
) {
    let Ok((camera, global, mut transform, mut projection)) = camera.single_mut() else { return };
    let Projection::Orthographic(ortho) = projection.as_mut() else { return };

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

    if !egui.wants_any_pointer_input() {
        if buttons.any_pressed([MouseButton::Right, MouseButton::Middle]) {
            let d = motion.delta * ortho.scale;
            transform.translation += Vec3::new(-d.x, d.y, 0.0);
        }

        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if notches != 0.0 {
            let cursor_world = windows
                .single()
                .ok()
                .and_then(Window::cursor_position)
                .and_then(|c| camera.viewport_to_world_2d(global, c).ok());
            let old = ortho.scale;
            ortho.scale = (old * 0.85f32.powf(notches)).clamp(MIN_SCALE, MAX_SCALE);
            // Keep the point under the cursor fixed while zooming.
            if let Some(p) = cursor_world {
                let cam = transform.translation.truncate();
                let new = p + (cam - p) * (ortho.scale / old);
                transform.translation = new.extend(transform.translation.z);
            }
        }
    }

    // Don't let the map be lost off-screen.
    let half = view.size / 2.0;
    transform.translation.x = transform.translation.x.clamp(-half.x, half.x);
    transform.translation.y = transform.translation.y.clamp(-half.y, half.y);
}
