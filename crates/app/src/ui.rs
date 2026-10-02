//! The egui overlay: time controls and the hex inspector.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, egui};
use content::BiomeId;
use sim_core::{Command, Date};

use crate::map_view::{MapView, Selection};
use crate::sim_thread::{SPEEDS, SimThread, Speed};

pub fn panels(
    mut contexts: EguiContexts,
    mut sim: ResMut<SimThread>,
    view: Res<MapView>,
    selection: Res<Selection>,
    mut reshape_to: Local<BiomeId>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let snap = sim.snapshot();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );

    egui::Panel::top("time").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            ui.strong(Date::from_tick(snap.tick).to_string());
            ui.separator();
            let current = sim.speed();
            for (i, (label, speed)) in SPEEDS.iter().enumerate() {
                let text = format!("{label} [{}]", if i == 0 { "Space".into() } else { i.to_string() });
                if ui.selectable_label(current == *speed, text).clicked() {
                    sim.set_speed(*speed);
                }
            }
            ui.separator();
            if current != Speed::Paused {
                ui.label(format!("{:.0} ticks/s", snap.ticks_per_second));
            }
        });
    });

    egui::Panel::right("inspector").default_size(220.0).show(&mut root, |ui| {
        ui.heading("Inspector");
        let world = &snap.world;
        match selection.0 {
            None => {
                ui.label("Click a hex to inspect it.");
            }
            Some(hex) => {
                let (col, row) = world.topology.offset(hex);
                let i = hex.index();
                let t = &world.terrain;
                let biome = view.registry.biome(t.biome[i]);
                ui.label(egui::RichText::new(&biome.name).strong().size(18.0));
                egui::Grid::new("hex").num_columns(2).show(ui, |ui| {
                    ui.label("Position");
                    ui.label(format!("{col}, {row}"));
                    ui.end_row();
                    ui.label("Elevation");
                    ui.label(format!("{:.0}%", t.elevation[i] * 100.0));
                    ui.end_row();
                    ui.label("Moisture");
                    ui.label(format!("{:.0}%", t.moisture[i] * 100.0));
                    ui.end_row();
                    ui.label("Temperature");
                    ui.label(format!("{:.0}%", t.temperature[i] * 100.0));
                    ui.end_row();
                    ui.label("Biome id");
                    ui.monospace(&biome.id);
                    ui.end_row();
                });

                ui.separator();
                ui.label(egui::RichText::new("God powers").strong());
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("reshape")
                        .selected_text(&view.registry.biome(*reshape_to).name)
                        .show_ui(ui, |ui| {
                            for (i, b) in view.registry.biomes().iter().enumerate() {
                                ui.selectable_value(&mut *reshape_to, BiomeId(i as u16), &b.name);
                            }
                        });
                    if ui.button("Reshape").clicked() {
                        sim.submit(Command::SetBiome { hex, biome: *reshape_to });
                    }
                });
                if sim.speed() == Speed::Paused {
                    ui.small("Takes effect on the next tick (unpause).");
                }
            }
        }
        ui.separator();
        ui.small(format!(
            "Seed {} · {}×{} hexes",
            world.seed,
            world.topology.width(),
            world.topology.height()
        ));
        ui.small("Drag with right mouse or WASD to pan, scroll to zoom.");
    });
    Ok(())
}

/// Space toggles pause; number keys pick a speed.
pub fn hotkeys(keys: Res<ButtonInput<KeyCode>>, egui: Res<EguiWantsInput>, mut sim: ResMut<SimThread>) {
    if egui.wants_any_keyboard_input() {
        return;
    }
    if keys.just_pressed(KeyCode::Space) {
        let next = if sim.speed() == Speed::Paused { SPEEDS[2].1 } else { Speed::Paused };
        sim.set_speed(next);
    }
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5];
    for (i, key) in digits.into_iter().enumerate() {
        if keys.just_pressed(key) {
            sim.set_speed(SPEEDS[i + 1].1);
        }
    }
}

/// Left click selects the hex under the cursor.
pub fn pick(
    buttons: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform)>,
    sim: Res<SimThread>,
    view: Res<MapView>,
    mut selection: ResMut<Selection>,
) {
    if !buttons.just_pressed(MouseButton::Left) || egui.wants_any_pointer_input() {
        return;
    }
    let Ok((camera, transform)) = camera.single() else { return };
    let Some(cursor) = windows.single().ok().and_then(Window::cursor_position) else { return };
    let Ok(pos) = camera.viewport_to_world_2d(transform, cursor) else { return };
    selection.0 = view.hex_at(&sim.snapshot().world, pos);
}
