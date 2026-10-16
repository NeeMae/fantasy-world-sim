//! The Esc menu: resume, save, load, start a new world or quit. The world
//! is paused while it's open.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_egui::egui;

use crate::saves::{self, SaveState};
use crate::sim_thread::{SimThread, Speed};
use crate::world_setup::WorldSettings;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Main,
    SaveAs,
    Load,
}

/// Something that needs a yes before it happens.
enum Confirm {
    /// Quit with unsaved changes.
    Quit,
    /// Load over unsaved changes.
    Load(PathBuf, String),
    /// Start a new world over unsaved changes.
    NewWorld,
    /// Delete a save.
    Delete(PathBuf, String),
    /// Save over an existing save of the same name.
    Overwrite,
}

/// What the menu asks the rest of the app to do.
pub enum Action {
    Load(PathBuf),
    NewWorld,
    Quit,
}

#[derive(Resource, Default)]
pub struct Menu {
    pub open: bool,
    page: Page,
    confirm: Option<Confirm>,
    /// The speed to go back to on resuming.
    resume: Option<Speed>,
    action: Option<Action>,
}

impl Menu {
    fn open(&mut self, sim: &mut SimThread) {
        self.open = true;
        self.page = Page::Main;
        self.confirm = None;
        self.resume = Some(sim.speed());
        sim.set_speed(Speed::Paused);
    }

    /// Closes the menu; `resume` restarts time at the speed it had.
    fn close(&mut self, sim: &mut SimThread, resume: bool) {
        self.open = false;
        self.confirm = None;
        if let Some(speed) = self.resume.take()
            && resume
        {
            sim.set_speed(speed);
        }
    }

    /// Esc: backs out of a question or a page, or opens or closes the menu.
    pub fn escape(&mut self, sim: &mut SimThread) {
        if !self.open {
            self.open(sim);
        } else if self.confirm.is_some() {
            self.confirm = None;
        } else if self.page != Page::Main {
            self.page = Page::Main;
        } else {
            self.close(sim, true);
        }
    }
}

/// Everything the menu acts on.
pub struct Context<'a> {
    pub sim: &'a mut SimThread,
    pub saves: &'a mut SaveState,
    pub settings: &'a mut WorldSettings,
}

const WIDTH: f32 = 340.0;

/// Draws the menu if it's open, returning anything it asks for.
pub fn show(ctx: &egui::Context, menu: &mut Menu, c: Context) -> Option<Action> {
    if !menu.open {
        return None;
    }
    let snap = c.sim.snapshot();
    let unsaved = c.saves.unsaved(&snap.world, snap.tick);

    // A modal dims the planet and keeps clicks off it while it's up.
    let title = match menu.page {
        Page::Main => "Paused",
        Page::SaveAs => "Save world",
        Page::Load => "Load world",
    };
    egui::Modal::new("menu".into()).backdrop_color(egui::Color32::from_black_alpha(150)).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.vertical_centered(|ui| ui.heading(title));
        ui.separator();
        if let Some(text) = menu.confirm.as_ref().map(confirm_text) {
            confirmation(ui, menu, text, c);
            return;
        }
        match menu.page {
            Page::Main => main_page(ui, menu, c, unsaved),
            Page::SaveAs => save_page(ui, menu, c),
            Page::Load => load_page(ui, menu, c, unsaved),
        }
    });
    menu.action.take()
}

fn big_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(text).size(16.0)).min_size(egui::vec2(WIDTH, 30.0)),
    )
}

fn status(ui: &mut egui::Ui, saves: &SaveState) {
    if let Some((message, ok)) = &saves.status {
        let color = if *ok { ui.visuals().weak_text_color() } else { egui::Color32::LIGHT_RED };
        ui.colored_label(color, message);
    }
}

fn main_page(ui: &mut egui::Ui, menu: &mut Menu, c: Context, unsaved: bool) {
    ui.vertical_centered(|ui| {
        if big_button(ui, true, "Resume").clicked() {
            menu.close(c.sim, true);
            return;
        }
        ui.add_space(6.0);
        let quick = if c.saves.name.trim().is_empty() {
            "Save…".to_string()
        } else {
            format!("Save “{}”", c.saves.name.trim())
        };
        if big_button(ui, !c.saves.busy(), &quick).on_hover_text("Ctrl+S").clicked() {
            if c.saves.name.trim().is_empty() {
                c.saves.name = format!("World {}", c.sim.snapshot().world.seed);
                menu.page = Page::SaveAs;
            } else {
                c.saves.save(c.sim, c.settings);
            }
        }
        if big_button(ui, true, "Save as…").clicked() {
            menu.page = Page::SaveAs;
        }
        if big_button(ui, true, "Load…").clicked() {
            c.saves.refresh();
            menu.page = Page::Load;
        }
        ui.add_space(6.0);
        if big_button(ui, true, "New random world")
            .on_hover_text("With the settings in the World panel and a fresh seed")
            .clicked()
        {
            if unsaved {
                menu.confirm = Some(Confirm::NewWorld);
            } else {
                new_world(menu, c);
                return;
            }
        }
        ui.add_space(6.0);
        if big_button(ui, true, "Quit").clicked() {
            if unsaved {
                menu.confirm = Some(Confirm::Quit);
            } else {
                menu.action = Some(Action::Quit);
            }
        }
        ui.add_space(4.0);
        if unsaved {
            ui.small("This world has unsaved changes.");
        }
        status(ui, c.saves);
    });
}

fn save_page(ui: &mut egui::Ui, menu: &mut Menu, c: Context) {
    ui.horizontal(|ui| {
        ui.label("Name");
        let edit = ui
            .add(egui::TextEdit::singleline(&mut c.saves.name).hint_text("Name").desired_width(WIDTH - 50.0));
        if c.saves.name.is_empty() {
            edit.request_focus();
        }
    });
    let file = saves::file_name(&c.saves.name);
    let exists = file
        .as_ref()
        .is_some_and(|f| c.saves.list().iter().any(|e| e.path.file_name().is_some_and(|n| n == f.as_str())));
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Existing saves").weak());
    let mut pick = None;
    let current = c.saves.name.clone();
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        let entries = c.saves.list();
        if entries.is_empty() {
            ui.small("None yet.");
        }
        for entry in entries {
            ui.horizontal(|ui| {
                if ui.selectable_label(entry.name == current, &entry.name).clicked() {
                    pick = Some(entry.name.clone());
                }
                if let Some(age) = entry.modified.and_then(|m| m.elapsed().ok()) {
                    ui.small(saves::format_age(age.as_secs()));
                }
            });
        }
    });
    if let Some(name) = pick {
        c.saves.name = name;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let can = file.is_some() && !c.saves.busy();
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.add_enabled(can, egui::Button::new(if exists { "Replace" } else { "Save" })).clicked()
            || (can && enter)
        {
            if exists {
                menu.confirm = Some(Confirm::Overwrite);
            } else {
                c.saves.save(c.sim, c.settings);
                menu.page = Page::Main;
            }
        }
        if ui.button("Back").clicked() {
            menu.page = Page::Main;
        }
    });
    status(ui, c.saves);
    ui.small(format!("Folder: {}", saves::saves_dir().display()));
}

fn load_page(ui: &mut egui::Ui, menu: &mut Menu, c: Context, unsaved: bool) {
    let mut load = None;
    let mut delete = None;
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        let entries = c.saves.list();
        if entries.is_empty() {
            ui.label("No saved worlds yet.");
        }
        for entry in entries {
            ui.horizontal(|ui| {
                if ui.button("Load").clicked() {
                    load = Some((entry.path.clone(), entry.name.clone()));
                }
                ui.label(egui::RichText::new(&entry.name).strong());
                if let Some(age) = entry.modified.and_then(|m| m.elapsed().ok()) {
                    ui.small(saves::format_age(age.as_secs()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Delete").clicked() {
                        delete = Some((entry.path.clone(), entry.name.clone()));
                    }
                });
            });
        }
    });
    if let Some((path, name)) = load {
        if unsaved {
            menu.confirm = Some(Confirm::Load(path, name));
        } else {
            do_load(menu, c, path, name);
            return;
        }
    }
    if let Some((path, name)) = delete {
        menu.confirm = Some(Confirm::Delete(path, name));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            menu.page = Page::Main;
        }
        if ui.button("Refresh").clicked() {
            c.saves.refresh();
        }
    });
    status(ui, c.saves);
    ui.small(format!("Folder: {}", saves::saves_dir().display()));
}

fn confirm_text(confirm: &Confirm) -> (String, &'static str) {
    match confirm {
        Confirm::Quit => ("Quit without saving? This world's unsaved changes will be lost.".into(), "Quit"),
        Confirm::Load(_, name) => {
            (format!("Load “{name}”? This world's unsaved changes will be lost."), "Load")
        }
        Confirm::NewWorld => {
            ("Start a new world? This world's unsaved changes will be lost.".into(), "New world")
        }
        Confirm::Delete(_, name) => (format!("Delete the save “{name}”? This can't be undone."), "Delete"),
        Confirm::Overwrite => ("Replace the existing save with this name?".into(), "Replace"),
    }
}

fn confirmation(ui: &mut egui::Ui, menu: &mut Menu, (text, yes): (String, &'static str), c: Context) {
    ui.add_space(4.0);
    ui.label(text);
    ui.add_space(8.0);
    let mut go = false;
    ui.horizontal(|ui| {
        go = ui.button(egui::RichText::new(yes).strong()).clicked();
        if ui.button("Cancel").clicked() {
            menu.confirm = None;
        }
    });
    if !go {
        return;
    }
    match menu.confirm.take() {
        Some(Confirm::Quit) => {
            menu.action = Some(Action::Quit);
        }
        Some(Confirm::Load(path, name)) => do_load(menu, c, path, name),
        Some(Confirm::NewWorld) => new_world(menu, c),
        Some(Confirm::Delete(path, _)) => c.saves.delete(&path),
        Some(Confirm::Overwrite) => {
            c.saves.save(c.sim, c.settings);
            menu.page = Page::Main;
        }
        None => {}
    }
}

/// Loads a save and closes the menu, leaving the loaded world paused.
fn do_load(menu: &mut Menu, c: Context, path: PathBuf, name: String) {
    c.saves.name = name;
    menu.action = Some(Action::Load(path));
    menu.close(c.sim, false);
}

/// Generates a new world with a fresh seed and closes the menu.
fn new_world(menu: &mut Menu, c: Context) {
    c.settings.seed = crate::world_setup::random_seed().to_string();
    menu.action = Some(Action::NewWorld);
    menu.close(c.sim, false);
}
