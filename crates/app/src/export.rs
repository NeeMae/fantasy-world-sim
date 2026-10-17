//! Exporting the map as a PNG image, drawn on a background thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use bevy::prelude::*;
use content::Registry;
use map_raster::RenderOptions;
use sim_core::World;

/// Where exported images go: an `exports` folder beside the saves.
pub fn exports_dir() -> PathBuf {
    crate::saves::saves_dir().with_file_name("exports")
}

/// Hex sizes offered for export, in pixels from centre to corner.
pub const SIZES: [(&str, f32); 4] = [("Small", 2.0), ("Medium", 4.0), ("Large", 6.0), ("Huge", 10.0)];

#[derive(Resource)]
pub struct ExportState {
    /// Index into [`SIZES`].
    pub size: usize,
    pub grid: bool,
    /// File name, without the extension.
    pub name: String,
    /// The last result to show: message, and whether it went well.
    pub status: Option<(String, bool)>,
    pending: Option<Mutex<Receiver<Result<PathBuf, String>>>>,
}

impl Default for ExportState {
    fn default() -> Self {
        ExportState { size: 1, grid: false, name: String::new(), status: None, pending: None }
    }
}

impl ExportState {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// The image size an export of `world` would have.
    pub fn dimensions(&self, world: &World) -> (u32, u32) {
        let layout = map_raster::HexLayout { size: SIZES[self.size.min(SIZES.len() - 1)].1 };
        let size = layout.image_size(&world.topology);
        (size.0, size.1)
    }

    /// Starts drawing `world` as `mode` into `exports/<name>.png`.
    pub fn start(&mut self, world: Arc<World>, registry: Arc<Registry>, mode: map_raster::MapMode) {
        if self.busy() {
            return;
        }
        let Some(file) = crate::saves::file_name(&self.name) else {
            self.status = Some(("Give the image a name first.".into(), false));
            return;
        };
        let opts = RenderOptions {
            layout: map_raster::HexLayout { size: SIZES[self.size.min(SIZES.len() - 1)].1 },
            mode,
            grid: self.grid,
            ..Default::default()
        };
        let path = exports_dir().join(file).with_extension("png");
        let (reply, result) = mpsc::channel();
        std::thread::spawn(move || {
            let write = || -> Result<PathBuf, String> {
                std::fs::create_dir_all(exports_dir()).map_err(|e| e.to_string())?;
                let img = map_raster::render(&world, &registry, &opts);
                image::save_buffer(&path, &img.rgba, img.width, img.height, image::ExtendedColorType::Rgba8)
                    .map_err(|e| e.to_string())?;
                Ok(path)
            };
            let _ = reply.send(write());
        });
        self.pending = Some(Mutex::new(result));
        self.status = Some(("Exporting…".into(), true));
    }
}

pub fn poll(mut state: ResMut<ExportState>) {
    let Some(pending) = &state.pending else { return };
    let result = match pending.lock().expect("export lock").try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => Err("the export stopped unexpectedly".into()),
    };
    state.pending = None;
    state.status = Some(match result {
        Ok(path) => (format!("Exported {}", path.display()), true),
        Err(e) => {
            warn!("export failed: {e}");
            (format!("Export failed: {e}"), false)
        }
    });
}
