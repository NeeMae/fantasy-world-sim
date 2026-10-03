//! The egui overlay: time controls, world generation, tools and the inspector.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, egui};
use content::{BiomeId, ReliefId};
use map_raster::MapMode;
use sim_core::{Command, Date, Paint};
use worldgen::EdgeStyle;

use crate::map_view::MapView;
use crate::sim_thread::{SPEEDS, SimThread, Speed};
use crate::tools::{Hover, MAX_BRUSH_RADIUS, MapModeSetting, PointerOverUi, Selection, Tool, ToolState};
use crate::world_setup::{
    ASPECTS, Climate, LARGE_WORLD, MAX_CANVAS, RegenerateRequest, SCALES, WorldSettings,
};

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
    mut map_mode: ResMut<MapModeSetting>,
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
            for (mode, label) in [
                (MapMode::Terrain, "Terrain"),
                (MapMode::Elevation, "Elevation"),
                (MapMode::Rainfall, "Rainfall"),
                (MapMode::Temperature, "Temperature"),
                (MapMode::Plates, "Plates"),
            ] {
                ui.selectable_value(&mut map_mode.0, mode, label).on_hover_text("M cycles map views");
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
            tools_section(ui, &mut tools, &view, &sim, snap.undo_redo);
            ui.separator();
            inspector_section(ui, &selection, &view, world);
            ui.separator();
            ui.small("Left click: use tool · Right-drag or WASD: pan · Wheel: zoom");
            ui.small("I: inspect · B: brush · [ ]: brush size · Ctrl+Z: undo · M: map view");
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
        "Current: seed {} · {}×{} hexes{}",
        world.seed,
        world.topology.width(),
        world.topology.height(),
        if world.topology.wrap() == sim_core::Wrap::X { " · wraps" } else { "" }
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

        ui.label("World size").on_hover_text("How much of the planet the map shows");
        ui.add(
            egui::Slider::new(&mut settings.world_size, 0.5..=6.0)
                .step_by(0.25)
                .custom_formatter(|v, _| world_size_name(v).to_string()),
        );
        ui.end_row();

        ui.label("Continents").on_hover_text("Larger gives fewer, bigger landmasses");
        ui.add(egui::Slider::new(&mut settings.continent_size, 0.5..=3.0).step_by(0.1));
        ui.end_row();

        ui.label("Scale")
            .on_hover_text("How finely the world is divided into hexes; the geography stays the same");
        ui.add_enabled_ui(settings.custom.is_none(), |ui| {
            egui::ComboBox::from_id_salt("scale").selected_text(SCALES[settings.scale].0).show_ui(ui, |ui| {
                for (i, (name, rows)) in SCALES.iter().enumerate() {
                    ui.selectable_value(&mut settings.scale, i, format!("{name} ({rows} rows)"));
                }
            });
        });
        ui.end_row();

        ui.label("Canvas");
        ui.add_enabled_ui(settings.custom.is_none(), |ui| {
            egui::ComboBox::from_id_salt("aspect").selected_text(ASPECTS[settings.aspect].0).show_ui(
                ui,
                |ui| {
                    for (i, (name, _)) in ASPECTS.iter().enumerate() {
                        ui.selectable_value(&mut settings.aspect, i, *name);
                    }
                },
            );
        });
        ui.end_row();

        ui.label("");
        let mut custom = settings.custom.is_some();
        if ui.checkbox(&mut custom, "Custom size").changed() {
            settings.custom = custom.then(|| settings.dimensions());
        }
        ui.end_row();
        if let Some((w, h)) = settings.custom.as_mut() {
            ui.label("");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(w).range(2..=MAX_CANVAS).speed(4));
                ui.label("×");
                ui.add(egui::DragValue::new(h).range(2..=MAX_CANVAS).speed(4));
            });
            ui.end_row();
        }

        ui.label("Edges");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut settings.edges, EdgeStyle::Open, "Open")
                .on_hover_text("Land runs off the map, as if it were part of a larger world");
            ui.selectable_value(&mut settings.edges, EdgeStyle::Ocean, "Ocean")
                .on_hover_text("The world is ringed by sea (north and south only when wrapping)");
        });
        ui.end_row();

        ui.label("Wrap");
        if ui
            .checkbox(&mut settings.wrap, "East–west")
            .on_hover_text("A globe: travel off the east edge and arrive in the west")
            .changed()
        {
            // A wrapped world is usually a whole planet.
            settings.climate = if settings.wrap { Climate::Globe } else { Climate::Regional };
        }
        ui.end_row();

        ui.label("Climate");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut settings.climate, Climate::Regional, "Regional")
                .on_hover_text("Cool north to warm south");
            ui.selectable_value(&mut settings.climate, Climate::Globe, "Globe")
                .on_hover_text("Pole to pole, with the equator across the middle");
        });
        ui.end_row();
    });

    let (w, h) = settings.dimensions();
    let hexes = w as u64 * h as u64;
    ui.small(format!("New world: {w}×{h} = {hexes} hexes"));
    if hexes > LARGE_WORLD {
        ui.colored_label(
            egui::Color32::from_rgb(230, 180, 80),
            "Very large: generation and drawing may be slow.",
        );
    }
    let valid = settings.seed.trim().parse::<u64>().is_ok();
    if ui.add_enabled(valid, egui::Button::new("Regenerate world")).clicked() {
        regenerate.write(RegenerateRequest);
    }
    if !valid {
        ui.colored_label(egui::Color32::LIGHT_RED, "Seed must be a whole number.");
    }
}

fn world_size_name(v: f64) -> String {
    let name = match v {
        v if v < 1.5 => "Region",
        v if v < 2.5 => "Subcontinent",
        v if v < 4.0 => "Continent",
        _ => "Planet",
    };
    format!("{v:.2} · {name}")
}

fn tools_section(
    ui: &mut egui::Ui,
    tools: &mut ToolState,
    view: &MapView,
    sim: &SimThread,
    undo_redo: (usize, usize),
) {
    ui.heading("Tools");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut tools.tool, Tool::Inspect, "Inspect [I]");
        ui.selectable_value(&mut tools.tool, Tool::Paint, "Paint [B]");
    });
    ui.horizontal(|ui| {
        let (undo, redo) = undo_redo;
        if ui.add_enabled(undo > 0, egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
            sim.submit(Command::Undo);
        }
        if ui
            .add_enabled(redo > 0, egui::Button::new("Redo"))
            .on_hover_text("Ctrl+Shift+Z or Ctrl+Y")
            .clicked()
        {
            sim.submit(Command::Redo);
        }
    });
    if tools.tool != Tool::Paint {
        return;
    }
    ui.add(egui::Slider::new(&mut tools.brush_radius, 0..=MAX_BRUSH_RADIUS).text("Brush radius"));
    ui.label("Relief");
    ui.horizontal_wrapped(|ui| {
        for (i, relief) in view.registry.reliefs().iter().enumerate() {
            ui.selectable_value(&mut tools.brush, Paint::Relief(ReliefId(i as u8)), &relief.name);
        }
    });
    ui.label("Biome");
    for (i, biome) in view.registry.biomes().iter().enumerate() {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            let c = biome.color;
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
            ui.selectable_value(&mut tools.brush, Paint::Biome(BiomeId(i as u16)), &biome.name);
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
            ("Rainfall", format!("{:.0}%", t.moisture[i] * 100.0)),
            ("Temperature", format!("{:.0}%", t.temperature[i] * 100.0)),
            (
                "Relief",
                format!(
                    "{} ({:.0}% rugged)",
                    view.registry.relief(t.relief[i]).name,
                    t.ruggedness[i] * 100.0
                ),
            ),
            ("Range", range_description(t.massif[i])),
            ("Plate", plate_description(world, i)),
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

fn range_description(massif: f32) -> String {
    match massif {
        m if m >= 0.6 => "Great range",
        m if m >= 0.35 => "Major range",
        m if m >= 0.1 => "Minor or old range",
        _ => "None",
    }
    .to_string()
}

fn plate_description(world: &sim_core::World, i: usize) -> String {
    let g = &world.geology;
    let id = g.plate[i];
    let kind = if g.plates[id as usize].continental { "continental" } else { "oceanic" };
    let activity = match g.stress[i] {
        s if s > 0.15 => ", colliding",
        s if s < -0.15 => ", rifting",
        _ => "",
    };
    format!("#{id} {kind}{activity}")
}

/// Space toggles pause; number keys pick a speed; I/B pick tools; [ ] resize the brush.
pub fn hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    egui: Res<EguiWantsInput>,
    mut sim: ResMut<SimThread>,
    mut tools: ResMut<ToolState>,
    mut map_mode: ResMut<MapModeSetting>,
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
    if keys.just_pressed(KeyCode::KeyM) {
        map_mode.0 = match map_mode.0 {
            MapMode::Terrain => MapMode::Elevation,
            MapMode::Elevation => MapMode::Rainfall,
            MapMode::Rainfall => MapMode::Temperature,
            MapMode::Temperature => MapMode::Plates,
            MapMode::Plates => MapMode::Terrain,
        };
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
    // A modifier released this same frame still counts as held, so a fast
    // tap of a shortcut isn't missed on a slow frame.
    let held = |pair: [KeyCode; 2]| keys.any_pressed(pair) || keys.any_just_released(pair);
    let ctrl = held([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = held([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        sim.submit(if shift { Command::Redo } else { Command::Undo });
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) {
        sim.submit(Command::Redo);
    }
}
