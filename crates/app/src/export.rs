//! Exporting the planet as a PNG image, drawn on a background thread.

use std::f64::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};

use bevy::prelude::*;
use content::Registry;
use map_raster::{Projection, RenderOptions, View};
use sim_core::World;

/// Where exported images go: an `exports` folder beside the saves.
pub fn exports_dir() -> PathBuf {
    crate::saves::saves_dir().with_file_name("exports")
}

/// Hex sizes offered for export, in pixels from centre to corner.
pub const SIZES: [(&str, f32); 4] = [("Small", 2.0), ("Medium", 4.0), ("Large", 6.0), ("Huge", 10.0)];

/// The longest side an exported image may have, in pixels.
pub const MAX_SIDE: u32 = 16384;

/// Share of the image's shorter side the globe fills (as on screen).
const GLOBE_FILL: f64 = 0.92;

/// What an export draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The whole planet as a flat map (equirectangular), twice as wide as tall.
    Map,
    /// The globe as seen from space, turned as it is on screen.
    Globe,
}

#[derive(Resource)]
pub struct ExportState {
    /// Index into [`SIZES`].
    pub size: usize,
    pub shape: Shape,
    pub grid: bool,
    /// File name, without the extension.
    pub name: String,
    /// The last result to show: message, and whether it went well.
    pub status: Option<(String, bool)>,
    pending: Option<Mutex<Receiver<Result<PathBuf, String>>>>,
}

impl Default for ExportState {
    fn default() -> Self {
        ExportState {
            size: 1,
            shape: Shape::Map,
            grid: false,
            name: String::new(),
            status: None,
            pending: None,
        }
    }
}

impl ExportState {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// The image size an export of `world` would have.
    pub fn dimensions(&self, world: &World) -> (u32, u32) {
        let hex = SIZES[self.size.min(SIZES.len() - 1)].1 as f64;
        // Radians per pixel that draw a hex `hex` pixels from centre to corner.
        let angle = world.topology.spacing() / (hex * 3f64.sqrt());
        match self.shape {
            Shape::Map => {
                let h = ((TAU / 2.0 / angle).round() as u32).clamp(1, MAX_SIDE / 2);
                (h * 2, h)
            }
            Shape::Globe => {
                let side = ((2.0 / angle / GLOBE_FILL).round() as u32).clamp(1, MAX_SIDE);
                (side, side)
            }
        }
    }

    /// Starts drawing `world` as `mode` into `exports/<name>.png`. A globe
    /// is turned to face like `view`; a map is centred on its longitude.
    pub fn start(
        &mut self,
        world: Arc<World>,
        registry: Arc<Registry>,
        mode: map_raster::MapMode,
        view: View,
    ) {
        if self.busy() {
            return;
        }
        let Some(file) = crate::saves::file_name(&self.name) else {
            self.status = Some(("Give the image a name first.".into(), false));
            return;
        };
        let (width, height) = self.dimensions(&world);
        let projection = match self.shape {
            Shape::Map => Projection::Map(View { lon: view.lon, lat: 0.0, zoom: 1.0 }),
            Shape::Globe => Projection::Globe(View { zoom: 1.0, ..view }),
        };
        let opts = RenderOptions { mode, grid: self.grid, ..Default::default() };
        let path = exports_dir().join(file).with_extension("png");
        let (reply, result) = mpsc::channel();
        std::thread::spawn(move || {
            let write = || -> Result<PathBuf, String> {
                std::fs::create_dir_all(exports_dir()).map_err(|e| e.to_string())?;
                let img = map_raster::render(&world, &registry, &opts, projection, width, height);
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
