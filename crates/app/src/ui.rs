//! The egui overlay: time controls, world generation, tools and the inspector.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, egui};
use content::{BiomeId, ReliefId};
use map_raster::MapMode;
use sim_core::{Command, Date, Paint};
use worldgen::EdgeStyle;

use crate::map_view::MapView;
use crate::saves::{self, SaveState};
use crate::sim_thread::{SPEEDS, SimThread, Speed};
use crate::tools::{Hover, MAX_BRUSH_RADIUS, MapModeSetting, PointerOverUi, Selection, Tool, ToolState};
use crate::world_setup::{
    ASPECTS, Climate, LARGE_WORLD, LoadRequest, MAX_CANVAS, RegenerateRequest, SCALES, WorldSettings,
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
    mut save_state: ResMut<SaveState>,
    mut load: MessageWriter<LoadRequest>,
    // Bumped by "More names" so the samples change.
    mut name_draw: Local<u64>,
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
            // Fixed-width slots, so the controls beside them don't shift as
            // the date grows or the rate changes.
            fixed_label(ui, "Year 9999999, month 12", Date::from_tick(snap.tick).to_string(), true);
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
                (MapMode::Fertility, "Fertility"),
                (MapMode::Currents, "Currents"),
                (MapMode::Plates, "Plates"),
            ] {
                ui.selectable_value(&mut map_mode.0, mode, label).on_hover_text("M cycles map views");
            }
            ui.separator();
            let rate = if current == Speed::Paused {
                String::new()
            } else {
                format!("{:.0} ticks/s", snap.ticks_per_second)
            };
            fixed_label(ui, "999999 ticks/s", rate, false);
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
            saves::section(ui, &mut save_state, &sim, &settings, &mut load);
            ui.separator();
            tools_section(ui, &mut tools, &view, &sim, snap.undo_redo);
            ui.separator();
            inspector_section(ui, &selection, &view, world);
            ui.separator();
            peoples_section(ui, &view, world, &mut name_draw);
            ui.separator();
            ui.small("Left click: use tool · Right-drag or WASD: pan · Wheel: zoom");
            ui.small("I: inspect · B: brush · [ ]: brush size · Ctrl+Z: undo · M: map view · Ctrl+S: save");
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

        ui.label("Volatility")
            .on_hover_text("How turbulent history will be: calm, long-lived empires or constant upheaval");
        ui.add(egui::Slider::new(&mut settings.volatility, 0.0..=1.0).custom_formatter(|v, _| {
            match v {
                v if v < 0.25 => "Calm",
                v if v < 0.55 => "Settled",
                v if v < 0.8 => "Turbulent",
                _ => "Chaotic",
            }
            .to_string()
        }));
        ui.end_row();

        ui.label("Sea level").on_hover_text("How much of the map lies under the sea");
        ui.add(
            egui::Slider::new(&mut settings.ocean, 0.2..=0.85)
                .step_by(0.01)
                .custom_formatter(|v, _| format!("{:.0}% sea", v * 100.0)),
        );
        ui.end_row();

        ui.label("Temperature").on_hover_text("Warmer or colder than an Earth-like world");
        ui.add(egui::Slider::new(&mut settings.temperature, -0.3..=0.3).step_by(0.01).custom_formatter(
            |v, _| {
                match v {
                    v if v < -0.2 => "Ice age",
                    v if v < -0.07 => "Cold",
                    v if v <= 0.07 => "Temperate",
                    v if v <= 0.2 => "Warm",
                    _ => "Hothouse",
                }
                .to_string()
            },
        ));
        ui.end_row();

        ui.label("Rainfall").on_hover_text("Wetter worlds have more forest, swamp and bigger rivers");
        ui.add(egui::Slider::new(&mut settings.rainfall, 0.3..=2.0).step_by(0.05).custom_formatter(
            |v, _| {
                match v {
                    v if v < 0.55 => "Arid",
                    v if v < 0.85 => "Dry",
                    v if v <= 1.2 => "Normal",
                    v if v <= 1.6 => "Wet",
                    _ => "Drenched",
                }
                .to_string()
            },
        ));
        ui.end_row();

        ui.label("Mountains").on_hover_text("How much mountain building the plates do");
        ui.add(egui::Slider::new(&mut settings.mountains, 0.0..=2.0).step_by(0.05).custom_formatter(
            |v, _| {
                match v {
                    v if v < 0.35 => "Worn flat",
                    v if v < 0.75 => "Gentle",
                    v if v <= 1.25 => "Normal",
                    v if v <= 1.65 => "Rugged",
                    _ => "Jagged",
                }
                .to_string()
            },
        ));
        ui.end_row();

        ui.label("Erosion").on_hover_text(
            "How much rivers and weather have worn the land: young and sharp, or old and carved",
        );
        ui.add(egui::Slider::new(&mut settings.erosion, 0.0..=2.0).step_by(0.05).custom_formatter(|v, _| {
            match v {
                v if v < 0.05 => "None",
                v if v < 0.7 => "Young",
                v if v <= 1.3 => "Normal",
                _ => "Ancient",
            }
            .to_string()
        }));
        ui.end_row();

        ui.label("Inland seas").on_hover_text("How many basins in the continents flood into enclosed seas");
        ui.add(egui::Slider::new(&mut settings.inland_seas, 0.0..=1.0).step_by(0.05).custom_formatter(
            |v, _| {
                match v {
                    v if v < 0.05 => "None",
                    v if v < 0.35 => "Few",
                    v if v < 0.7 => "Some",
                    _ => "Many",
                }
                .to_string()
            },
        ));
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
            ("River", river_description(t, i)),
            ("Fertility", format!("{:.0}%", t.fertility(&view.registry, i) * 100.0)),
            ("Travel cost", format!("×{:.1}", t.travel_cost(&view.registry, i))),
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

/// Text in a slot as wide as `widest` would be, so whatever follows it
/// stays put while the text changes.
fn fixed_label(ui: &mut egui::Ui, widest: &str, text: String, strong: bool) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = if strong { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let width = ui.fonts_mut(|f| f.layout_no_wrap(widest.to_string(), font.clone(), color).size().x);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, ui.spacing().interact_size.y), egui::Sense::hover());
    // Clipped, so even an absurdly long date can't spill over its neighbours.
    ui.painter().with_clip_rect(rect).text(rect.left_center(), egui::Align2::LEFT_CENTER, text, font, color);
}

/// The cultures in the loaded content, each with sample names in its own
/// language. Peoples aren't placed in the world yet (that's Phase 2).
fn peoples_section(ui: &mut egui::Ui, view: &MapView, world: &sim_core::World, draw: &mut u64) {
    egui::CollapsingHeader::new(egui::RichText::new("Peoples").heading()).default_open(false).show(
        ui,
        |ui| {
            let registry = &view.registry;
            if registry.cultures().is_empty() {
                ui.label("No cultures in the loaded content.");
                return;
            }
            for (i, culture) in registry.cultures().iter().enumerate() {
                let race = registry.race(culture.race_id);
                let mut rng =
                    sim_core::rng::stream(world.seed, *draw, i as u64, sim_core::rng::purpose::NAMES);
                let mut sample = |kind| {
                    (0..4)
                        .map(|_| names::generate(&culture.def.language, kind, &mut rng))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let (people, places) = (sample(names::NameKind::Person), sample(names::NameKind::Place));
                ui.label(egui::RichText::new(format!("{} ({})", culture.def.name, race.name)).strong())
                    .on_hover_text(format!(
                        "{}\n{} (lifespan ~{} years)",
                        culture.def.description, race.description, race.lifespan
                    ));
                ui.small(format!("People: {people}"));
                ui.small(format!("Places: {places}"));
                ui.small(format!("Titles: {}", culture.def.title_tiers.join(" › ")));
                ui.add_space(4.0);
            }
            if ui.button("More names").clicked() {
                *draw += 1;
            }
        },
    );
}

fn river_description(t: &sim_core::Terrain, i: usize) -> String {
    match t.river(i) {
        r if r <= 0.0 => "None".into(),
        r if r < 2.0 => "Stream".into(),
        r if r < 4.0 => "River".into(),
        _ => "Great river".into(),
    }
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
    mut save_state: ResMut<SaveState>,
    settings: Res<WorldSettings>,
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
            MapMode::Temperature => MapMode::Fertility,
            MapMode::Fertility => MapMode::Currents,
            MapMode::Currents => MapMode::Plates,
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
    if ctrl && keys.just_pressed(KeyCode::KeyS) {
        if save_state.name.trim().is_empty() {
            save_state.name = format!("World {}", sim.snapshot().world.seed);
        }
        save_state.save(&sim, &settings);
    }
}
