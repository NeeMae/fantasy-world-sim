//! Saving and loading worlds from the app.
//!
//! Saves live in a `saves` folder in the user's data directory (override
//! with the `FWS_SAVES` environment variable), one `.world` file each.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use bevy::prelude::*;

use crate::sim_thread::SimThread;
use crate::world_setup::WorldSettings;

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
pub fn file_name(name: &str) -> Option<String> {
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
    /// The world state the last save (or load) captured, and the one a
    /// save in progress is capturing: seed, tick and terrain revision.
    saved: Option<(u64, u64, u64)>,
    saving: Option<(u64, u64, u64)>,
}

/// What identifies a world's state for "unsaved changes".
/// (The snapshot's world is only republished when terrain changes, so the
/// tick comes separately.)
fn mark(world: &sim_core::World, tick: u64) -> (u64, u64, u64) {
    (world.seed, tick, world.terrain_revision)
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

    /// Whether the world has changed (or time has passed) since it was last
    /// saved or loaded. A fresh, untouched world doesn't count: its seed
    /// and settings make it again.
    pub fn unsaved(&self, world: &sim_core::World, tick: u64) -> bool {
        let fresh = tick == 0 && world.terrain_revision == 0;
        !fresh && self.saved != Some(mark(world, tick))
    }

    /// Records that `world` is what's on disk (just loaded).
    pub fn mark_saved(&mut self, world: &sim_core::World) {
        self.saved = Some(mark(world, world.tick));
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
        // Saving pauses nothing: the state captured is the one between
        // ticks when the sim thread gets the request, about now.
        let snap = sim.snapshot();
        self.saving = Some(mark(&snap.world, snap.tick));
        self.pending = Some(Mutex::new(sim.save(saves_dir().join(file), settings)));
        self.status = Some(("Saving…".into(), true));
    }

    /// Deletes a save.
    pub fn delete(&mut self, path: &Path) {
        match std::fs::remove_file(path) {
            Ok(()) => self.report(format!("Deleted {}", display_name(path)), true),
            Err(e) => self.report(format!("Could not delete {}: {e}", display_name(path)), false),
        }
        self.refresh();
    }

    /// Saved worlds, newest first. Cached until something is saved.
    pub fn list(&mut self) -> &[SaveEntry] {
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
        Ok(path) => {
            state.saved = state.saving.take();
            state.report(format!("Saved {}", display_name(&path)), true)
        }
        Err(e) => state.report(format!("Save failed: {e}"), false),
    }
}

pub fn format_age(secs: u64) -> String {
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}
