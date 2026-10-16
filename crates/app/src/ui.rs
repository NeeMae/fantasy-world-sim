//! The egui overlay: time controls, world generation, tools and the inspector.

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, egui};
use content::Registry;
use content::{BiomeId, ReliefId};
use map_raster::MapMode;
use sim_core::{Command, Date, Paint};

use crate::menu::{Action, Menu};
use crate::planet_view::{Outline, PlanetView, describe_position};
use crate::saves::SaveState;
use crate::sim_thread::{SPEEDS, SimThread, Speed};
use crate::tools::{Hover, MAX_BRUSH_RADIUS, MapModeSetting, Selection, Tool, ToolState};
use crate::world_setup::{Content, DETAILS, LARGE_WORLD, LoadRequest, RegenerateRequest, WorldSettings};

pub fn panels(
    mut contexts: EguiContexts,
    mut sim: ResMut<SimThread>,
    content: Res<Content>,
    mut selection: ResMut<Selection>,
    mut hover: ResMut<Hover>,
    mut planet: ResMut<PlanetView>,
    mut tools: ResMut<ToolState>,
    mut settings: ResMut<WorldSettings>,
    mut regenerate: MessageWriter<RegenerateRequest>,
    mut map_mode: ResMut<MapModeSetting>,
    mut save_state: ResMut<SaveState>,
    mut load: MessageWriter<LoadRequest>,
    mut exit: MessageWriter<AppExit>,
    mut menu: ResMut<Menu>,
    // Bumped by "More names" so the samples change.
    mut name_draw: Local<u64>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let registry = &content.0;
    let snap = sim.snapshot();
    let world = &snap.world;
    let mut root = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );

    egui::Panel::top("time").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            if ui.button("☰ Menu").on_hover_text("Save, load, new world, quit (Esc)").clicked() {
                menu.escape(&mut sim);
            }
            ui.separator();
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
            ui.selectable_value(&mut planet.globe, true, "Globe")
                .on_hover_text("The planet seen from space (G switches)");
            ui.selectable_value(&mut planet.globe, false, "Map")
                .on_hover_text("The whole planet laid flat (G switches)");
            ui.toggle_value(&mut planet.grid, "Grid")
                .on_hover_text("Lines between hexes, when they're big enough");
            ui.separator();
            let rate = if current == Speed::Paused {
                String::new()
            } else {
                format!("{:.0} ticks/s", snap.ticks_per_second)
            };
            fixed_label(ui, "999999 ticks/s", rate, false);
            if let Some(hex) = hover.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let biome = registry.biome(world.terrain.biome[hex.index()]);
                    ui.label(format!("{} · {}", biome.name, describe_position(&world.topology, hex)));
                });
            }
        });
    });

    egui::Panel::right("side").default_size(250.0).show(&mut root, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            world_section(ui, &mut settings, &mut regenerate, world);
            ui.separator();
            tools_section(ui, &mut tools, registry, &sim, snap.undo_redo);
            ui.separator();
            inspector_section(ui, &selection, registry, world);
            ui.separator();
            peoples_section(ui, registry, world, &mut name_draw);
            ui.separator();
            ui.small("Left click: use tool · Right-drag or WASD: turn · Wheel or +/−: zoom");
            ui.small("I: inspect · B: brush · [ ]: brush size · Ctrl+Z: undo · M: map view · G: globe/map · Home: recentre · Ctrl+S: save · Esc: menu");
        });
    });

    // The planet fills the rest, with outlines for what's hovered, what the
    // brush would cover, and what's selected.
    let hovered: Vec<_> = hover.0.map(|h| tools.affected_hexes(world, h)).unwrap_or_default();
    let selected: Vec<_> = selection.0.into_iter().collect();
    let outlines = [
        Outline {
            hexes: &hovered,
            color: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170),
            width: 1.0,
        },
        Outline { hexes: &selected, color: egui::Color32::from_rgb(255, 214, 90), width: 2.0 },
    ];
    let pointer = egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(12, 14, 22)))
        .show(&mut root, |ui| planet.show(ui, world, registry, map_mode.0, &outlines))
        .inner;
    let world = world.clone();
    let context = crate::menu::Context { sim: &mut sim, saves: &mut save_state, settings: &mut settings };
    match crate::menu::show(ctx, &mut menu, context) {
        Some(Action::Load(path)) => {
            load.write(LoadRequest(path));
        }
        Some(Action::NewWorld) => {
            regenerate.write(RegenerateRequest);
        }
        Some(Action::Quit) => {
            exit.write(AppExit::Success);
        }
        None => {}
    }
    // The menu's backdrop covers the planet, so it sees no pointer then.
    hover.0 = pointer.hover.filter(|_| !menu.open);
    if !menu.open {
        tools.apply(&pointer, &sim, &world, &mut selection);
    }
    Ok(())
}

fn world_section(
    ui: &mut egui::Ui,
    settings: &mut WorldSettings,
    regenerate: &mut MessageWriter<RegenerateRequest>,
    world: &sim_core::World,
) {
    ui.heading("World");
    ui.small(format!("Current: seed {} · {} hexes", world.seed, world.topology.len()));
    egui::Grid::new("worldgen").num_columns(2).show(ui, |ui| {
        ui.label("Seed");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut settings.seed).desired_width(100.0));
            if ui.button("Random").clicked() {
                settings.seed = crate::world_setup::random_seed().to_string();
            }
        });
        ui.end_row();

        ui.label("Planet size").on_hover_text("Bigger planets hold more, smaller continents");
        ui.add(
            egui::Slider::new(&mut settings.world_size, 2.0..=10.0)
                .step_by(0.25)
                .custom_formatter(|v, _| world_size_name(v).to_string()),
        );
        ui.end_row();

        ui.label("Continents").on_hover_text("Larger gives fewer, bigger landmasses");
        ui.add(egui::Slider::new(&mut settings.continent_size, 0.5..=3.0).step_by(0.1));
        ui.end_row();

        ui.label("Detail")
            .on_hover_text("How finely the planet is divided into hexes; the geography stays the same");
        egui::ComboBox::from_id_salt("detail")
            .selected_text(DETAILS[settings.detail.min(DETAILS.len() - 1)].0)
            .show_ui(ui, |ui| {
                for (i, (name, f)) in DETAILS.iter().enumerate() {
                    let hexes = 10 * (*f as u64) * (*f as u64) + 2;
                    ui.selectable_value(&mut settings.detail, i, format!("{name} ({}k hexes)", hexes / 1000));
                }
            });
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

        ui.label("Rivers").on_hover_text(
            "How many streams count as rivers: only the great rivers, or every sizeable stream",
        );
        ui.add(egui::Slider::new(&mut settings.rivers, 0.0..=1.0).step_by(0.05).custom_formatter(|v, _| {
            match v {
                v if v < 0.2 => "Great only",
                v if v < 0.4 => "Few",
                v if v <= 0.6 => "Normal",
                v if v <= 0.8 => "Many",
                _ => "Every stream",
            }
            .to_string()
        }));
        ui.end_row();

        ui.label("Lakes").on_hover_text("How big lakes grow before evaporation balances their inflow");
        ui.add(egui::Slider::new(&mut settings.lakes, 0.0..=2.0).step_by(0.05).custom_formatter(|v, _| {
            match v {
                v if v < 0.05 => "None",
                v if v < 0.7 => "Small",
                v if v <= 1.3 => "Normal",
                _ => "Big",
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
    });

    let hexes = settings.hexes();
    ui.small(format!("New world: {hexes} hexes"));
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
        v if v < 3.0 => "Small",
        v if v < 5.0 => "Earth-like",
        v if v < 7.5 => "Large",
        _ => "Vast",
    };
    format!("{v:.2} · {name}")
}

fn tools_section(
    ui: &mut egui::Ui,
    tools: &mut ToolState,
    registry: &Registry,
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
        for (i, relief) in registry.reliefs().iter().enumerate() {
            ui.selectable_value(&mut tools.brush, Paint::Relief(ReliefId(i as u8)), &relief.name);
        }
    });
    ui.label("Biome");
    for (i, biome) in registry.biomes().iter().enumerate() {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            let c = biome.color;
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
            ui.selectable_value(&mut tools.brush, Paint::Biome(BiomeId(i as u16)), &biome.name);
        });
    }
}

fn inspector_section(ui: &mut egui::Ui, selection: &Selection, registry: &Registry, world: &sim_core::World) {
    ui.heading("Inspector");
    let Some(hex) = selection.0 else {
        ui.label("Select a hex with the Inspect tool.");
        return;
    };
    let i = hex.index();
    let t = &world.terrain;
    let biome = registry.biome(t.biome[i]);
    ui.label(egui::RichText::new(&biome.name).strong().size(18.0));
    egui::Grid::new("hex").num_columns(2).show(ui, |ui| {
        for (label, value) in [
            ("Position", describe_position(&world.topology, hex)),
            ("Elevation", format!("{:.0}%", t.elevation[i] * 100.0)),
            ("Rainfall", format!("{:.0}%", t.moisture[i] * 100.0)),
            ("Temperature", format!("{:.0}%", t.temperature[i] * 100.0)),
            (
                "Relief",
                format!("{} ({:.0}% rugged)", registry.relief(t.relief[i]).name, t.ruggedness[i] * 100.0),
            ),
            ("Range", range_description(t.massif[i])),
            ("River", river_description(t, i)),
            ("Fertility", format!("{:.0}%", t.fertility(registry, i) * 100.0)),
            ("Travel cost", format!("×{:.1}", t.travel_cost(registry, i))),
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
fn peoples_section(ui: &mut egui::Ui, registry: &Registry, world: &sim_core::World, draw: &mut u64) {
    egui::CollapsingHeader::new(egui::RichText::new("Peoples").heading()).default_open(false).show(
        ui,
        |ui| {
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
    mut planet: ResMut<PlanetView>,
    mut menu: ResMut<Menu>,
    time: Res<Time>,
) {
    // Esc works even while typing (it backs out of the menu's pages).
    if keys.just_pressed(KeyCode::Escape) {
        menu.escape(&mut sim);
    }
    if menu.open || egui.wants_any_keyboard_input() {
        return;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        planet.globe = !planet.globe;
    }
    if keys.just_pressed(KeyCode::Home) {
        planet.reset();
    }
    // Turning with the keys, unless Ctrl is down (Ctrl+S saves).
    let ctrl_down = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if !ctrl_down {
        let step = 500.0 * time.delta_secs();
        let mut d = (0.0, 0.0);
        for (pair, (dx, dy)) in [
            ([KeyCode::KeyW, KeyCode::ArrowUp], (0.0, 1.0)),
            ([KeyCode::KeyS, KeyCode::ArrowDown], (0.0, -1.0)),
            ([KeyCode::KeyA, KeyCode::ArrowLeft], (1.0, 0.0)),
            ([KeyCode::KeyD, KeyCode::ArrowRight], (-1.0, 0.0)),
        ] {
            if keys.any_pressed(pair) {
                d = (d.0 + dx * step, d.1 + dy * step);
            }
        }
        if d != (0.0, 0.0) {
            planet.pan(d.0, d.1);
        }
        if keys.any_pressed([KeyCode::Equal, KeyCode::NumpadAdd]) {
            planet.zoom_by((1.5 * time.delta_secs() as f64).exp());
        }
        if keys.any_pressed([KeyCode::Minus, KeyCode::NumpadSubtract]) {
            planet.zoom_by((-1.5 * time.delta_secs() as f64).exp());
        }
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
