//! The egui overlay: time controls, world generation, tools and the inspector.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, egui};
use content::BiomeId;
use sim_core::Date;

use crate::map_view::MapView;
use crate::sim_thread::{SPEEDS, SimThread, Speed};
use crate::tools::{Hover, MAX_BRUSH_RADIUS, PointerOverUi, Selection, Tool, ToolState};
use crate::world_setup::{RegenerateRequest, SIZES, WorldSettings};

pub fn panels(
    mut contexts: EguiContexts,
    mut sim: ResMut<SimThread>,
    view: Res<MapView>,
    selection: Res<Selection>,
    hover: Res<Hover>,
    mut tools: ResMut<ToolState>,
    mut settings: ResMut<WorldSettings>,
    mut regenerate: MessageWriter<RegenerateRequest>,
    mut over_ui: ResMut<PointerOverUi>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let snap = sim.snapshot();
    let world = &snap.world;
    let mut root = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );

    let top = egui::Panel::top("time").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            ui.strong(Date::from_tick(snap.tick).to_string());
            ui.separator();
            let current = sim.speed();
            for (i, (label, speed)) in SPEEDS.iter().enumerate() {
                let key = if i == 0 { "Space".to_string() } else { i.to_string() };
                if ui.selectable_label(current == *speed, format!("{label} [{key}]")).clicked() {
                    sim.set_speed(*speed);
                }
            }
            ui.separator();
            if current != Speed::Paused {
                ui.label(format!("{:.0} ticks/s", snap.ticks_per_second));
            }
            if let Some(hex) = hover.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (col, row) = world.topology.offset(hex);
                    let biome = view.registry.biome(world.terrain.biome[hex.index()]);
                    ui.label(format!("{} ({col}, {row})", biome.name));
                });
            }
        });
    });

    let side = egui::Panel::right("side").default_size(250.0).show(&mut root, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            world_section(ui, &mut settings, &mut regenerate, world);
            ui.separator();
            tools_section(ui, &mut tools, &view);
            ui.separator();
            inspector_section(ui, &selection, &view, world);
            ui.separator();
            ui.small("Left click: use tool · Right-drag or WASD: pan · Wheel: zoom");
            ui.small("I: inspect · B: brush · [ ]: brush size");
        });
    });

    let to_rect = |r: egui::Rect| Rect::new(r.min.x, r.min.y, r.max.x, r.max.y);
    over_ui.panels = vec![to_rect(top.response.rect), to_rect(side.response.rect)];
    over_ui.busy = ctx.is_pointer_over_egui() || ctx.egui_is_using_pointer() || ctx.any_popup_open();
    Ok(())
}

fn world_section(
    ui: &mut egui::Ui,
    settings: &mut WorldSettings,
    regenerate: &mut MessageWriter<RegenerateRequest>,
    world: &sim_core::World,
) {
    ui.heading("World");
    ui.small(format!(
        "Current: seed {} · {}×{} hexes",
        world.seed,
        world.topology.width(),
        world.topology.height()
    ));
    egui::Grid::new("worldgen").num_columns(2).show(ui, |ui| {
        ui.label("Seed");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut settings.seed).desired_width(100.0));
            if ui.button("Random").clicked() {
                settings.seed = crate::world_setup::random_seed().to_string();
            }
        });
        ui.end_row();
        ui.label("Size");
        egui::ComboBox::from_id_salt("size").selected_text(SIZES[settings.size].0).show_ui(ui, |ui| {
            for (i, (name, w, h)) in SIZES.iter().enumerate() {
                ui.selectable_value(&mut settings.size, i, format!("{name} ({w}×{h})"));
            }
        });
        ui.end_row();
    });
    let valid = settings.seed.trim().parse::<u64>().is_ok();
    if ui.add_enabled(valid, egui::Button::new("Regenerate world")).clicked() {
        regenerate.write(RegenerateRequest);
    }
    if !valid {
        ui.colored_label(egui::Color32::LIGHT_RED, "Seed must be a whole number.");
    }
}

fn tools_section(ui: &mut egui::Ui, tools: &mut ToolState, view: &MapView) {
    ui.heading("Tools");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut tools.tool, Tool::Inspect, "Inspect [I]");
        ui.selectable_value(&mut tools.tool, Tool::Paint, "Paint [B]");
    });
    if tools.tool != Tool::Paint {
        return;
    }
    ui.add(egui::Slider::new(&mut tools.brush_radius, 0..=MAX_BRUSH_RADIUS).text("Brush radius"));
    ui.label("Terrain");
    for (i, biome) in view.registry.biomes().iter().enumerate() {
        let id = BiomeId(i as u16);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            let c = biome.color;
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
            ui.selectable_value(&mut tools.brush_biome, id, &biome.name);
        });
    }
}

fn inspector_section(ui: &mut egui::Ui, selection: &Selection, view: &MapView, world: &sim_core::World) {
    ui.heading("Inspector");
    let Some(hex) = selection.0 else {
        ui.label("Select a hex with the Inspect tool.");
        return;
    };
    let (col, row) = world.topology.offset(hex);
    let i = hex.index();
    let t = &world.terrain;
    let biome = view.registry.biome(t.biome[i]);
    ui.label(egui::RichText::new(&biome.name).strong().size(18.0));
    egui::Grid::new("hex").num_columns(2).show(ui, |ui| {
        for (label, value) in [
            ("Position", format!("{col}, {row}")),
            ("Elevation", format!("{:.0}%", t.elevation[i] * 100.0)),
            ("Moisture", format!("{:.0}%", t.moisture[i] * 100.0)),
            ("Temperature", format!("{:.0}%", t.temperature[i] * 100.0)),
        ] {
            ui.label(label);
            ui.label(value);
            ui.end_row();
        }
        ui.label("Biome id");
        ui.monospace(&biome.id);
        ui.end_row();
    });
}

/// Space toggles pause; number keys pick a speed; I/B pick tools; [ ] resize the brush.
pub fn hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    egui: Res<EguiWantsInput>,
    mut sim: ResMut<SimThread>,
    mut tools: ResMut<ToolState>,
) {
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
    if keys.just_pressed(KeyCode::KeyI) {
        tools.tool = Tool::Inspect;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        tools.tool = Tool::Paint;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        tools.brush_radius = tools.brush_radius.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        tools.brush_radius = (tools.brush_radius + 1).min(MAX_BRUSH_RADIUS);
    }
}
