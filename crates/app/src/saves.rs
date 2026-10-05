//! Saving and loading worlds from the app.
//!
//! Saves live in a `saves` folder in the user's data directory (override
//! with the `FWS_SAVES` environment variable), one `.world` file each.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use bevy::prelude::*;
use bevy_egui::egui;

use crate::sim_thread::SimThread;
use crate::world_setup::{LoadRequest, WorldSettings};

pub const EXTENSION: &str = "world";

/// Where saves go.
pub fn saves_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FWS_SAVES") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".local/share")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("fantasy-world-sim").join("saves")
}

/// The save's name as shown in the list: its file name without extension.
pub fn display_name(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// Keeps file names portable: letters, digits, spaces, `-` and `_` only.
fn file_name(name: &str) -> Option<String> {
    let clean: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' })
        .collect();
    let clean = clean.trim().to_string();
    (!clean.is_empty()).then(|| format!("{clean}.{EXTENSION}"))
}

pub struct SaveEntry {
    pub name: String,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

#[derive(Resource, Default)]
pub struct SaveState {
    /// The name to save under.
    pub name: String,
    /// The last result to show: message, and whether it went well.
    pub status: Option<(String, bool)>,
    pending: Option<Mutex<Receiver<Result<PathBuf, String>>>>,
    list: Option<Vec<SaveEntry>>,
}

impl SaveState {
    pub fn report(&mut self, message: String, ok: bool) {
        if !ok {
            warn!("{message}");
        }
        self.status = Some((message, ok));
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Starts saving the current world under [`SaveState::name`].
    pub fn save(&mut self, sim: &SimThread, settings: &WorldSettings) {
        if self.busy() {
            return;
        }
        let Some(file) = file_name(&self.name) else {
            self.report("Give the save a name first.".into(), false);
            return;
        };
        let settings = ron::to_string(settings).unwrap_or_default();
        self.pending = Some(Mutex::new(sim.save(saves_dir().join(file), settings)));
        self.status = Some(("Saving…".into(), true));
    }

    /// Saved worlds, newest first. Cached until something is saved.
    fn list(&mut self) -> &[SaveEntry] {
        self.list.get_or_insert_with(|| {
            let mut list: Vec<SaveEntry> = std::fs::read_dir(saves_dir())
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == EXTENSION))
                .map(|path| SaveEntry {
                    name: display_name(&path),
                    modified: path.metadata().and_then(|m| m.modified()).ok(),
                    path,
                })
                .collect();
            list.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
            list
        })
    }

    pub fn refresh(&mut self) {
        self.list = None;
    }
}

/// Collects finished saves.
pub fn poll(mut state: ResMut<SaveState>) {
    let Some(pending) = &state.pending else {
        return;
    };
    let Ok(result) = pending.lock().expect("never poisoned").try_recv() else {
        return;
    };
    state.pending = None;
    state.refresh();
    match result {
        Ok(path) => state.report(format!("Saved {}", display_name(&path)), true),
        Err(e) => state.report(format!("Save failed: {e}"), false),
    }
}

pub fn section(
    ui: &mut egui::Ui,
    state: &mut SaveState,
    sim: &SimThread,
    settings: &WorldSettings,
    load: &mut MessageWriter<LoadRequest>,
) {
    egui::CollapsingHeader::new(egui::RichText::new("Saves").heading()).default_open(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.name).hint_text("Name").desired_width(130.0));
            let can_save = !state.busy() && !state.name.trim().is_empty();
            if ui.add_enabled(can_save, egui::Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
                state.save(sim, settings);
            }
        });
        if let Some(file) = file_name(&state.name)
            && state.list().iter().any(|e| e.path.file_name().is_some_and(|f| f == file.as_str()))
        {
            ui.small("Saving will replace the existing save with this name.");
        }
        if let Some((message, ok)) = &state.status {
            let color = if *ok { ui.visuals().weak_text_color() } else { egui::Color32::LIGHT_RED };
            ui.colored_label(color, message);
        }
        ui.separator();
        let mut chosen = None;
        let entries = state.list();
        if entries.is_empty() {
            ui.small("No saved worlds yet.");
        }
        for entry in entries {
            ui.horizontal(|ui| {
                if ui.button("Load").clicked() {
                    chosen = Some((entry.path.clone(), entry.name.clone()));
                }
                ui.label(&entry.name);
                if let Some(age) = entry.modified.and_then(|m| m.elapsed().ok()) {
                    ui.small(format_age(age.as_secs()));
                }
            });
        }
        if let Some((path, name)) = chosen {
            state.name = name;
            load.write(LoadRequest(path));
        }
        if ui.small_button("Refresh").clicked() {
            state.refresh();
        }
        ui.small(format!("Folder: {}", saves_dir().display()));
    });
}

fn format_age(secs: u64) -> String {
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}
