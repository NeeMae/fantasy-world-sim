//! Generating worlds in the background, with a live view of each stage.
//!
//! The generator runs on its own thread and reports each stage as it goes,
//! with pictures of the world so far (drawn on that thread too). The app
//! shows them in a window and swaps the new world in when it's done; the
//! old world stays up, and running, until then.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy_egui::egui;
use content::Registry;
use map_raster::{HexLayout, MapMode, RenderOptions};
use sim_core::World;
use worldgen::{PreviewView, Stage, WorldGenParams};

/// Width in pixels the previews are drawn at.
const PREVIEW_WIDTH: f32 = 720.0;
/// Least time between previews, so drawing them doesn't slow generation.
const PREVIEW_EVERY: Duration = Duration::from_millis(120);

enum Message {
    Progress { stage: Stage, fraction: f32 },
    Preview { width: u32, height: u32, rgba: Vec<u8> },
    Done(Box<Result<World, String>>),
}

struct Job {
    messages: Mutex<Receiver<Message>>,
    stage: Stage,
    fraction: f32,
    started: Instant,
    /// A new preview not yet uploaded to egui.
    fresh: Option<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
}

/// The world being generated, if any.
#[derive(Resource, Default)]
pub struct Generation {
    job: Option<Job>,
}

impl Generation {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Starts generating; a generation already running is abandoned.
    pub fn start(&mut self, params: WorldGenParams, registry: Arc<Registry>) {
        let (send, messages) = mpsc::channel();
        std::thread::Builder::new()
            .name("worldgen".into())
            .spawn(move || {
                let mut last_preview: Option<Instant> = None;
                let result = worldgen::generate_with(&params, &registry, &mut |p| {
                    let _ = send.send(Message::Progress { stage: p.stage, fraction: p.fraction });
                    let Some((world, view)) = p.preview else { return };
                    // Always show a stage's last picture; throttle the rest.
                    let due = last_preview.is_none_or(|t| t.elapsed() >= PREVIEW_EVERY) || p.fraction >= 1.0;
                    if due {
                        last_preview = Some(Instant::now());
                        let _ = send.send(preview(world, &registry, view));
                    }
                });
                let _ = send.send(Message::Done(Box::new(result.map_err(|e| e.to_string()))));
            })
            .expect("failed to start world generation");
        self.job = Some(Job {
            messages: Mutex::new(messages),
            stage: Stage::Plates,
            fraction: 0.0,
            started: Instant::now(),
            fresh: None,
            texture: None,
        });
    }

    /// Takes in progress reports; returns the world once it's finished.
    pub fn poll(&mut self) -> Option<Result<World, String>> {
        let job = self.job.as_mut()?;
        let mut done = None;
        for message in job.messages.lock().expect("never poisoned").try_iter() {
            match message {
                Message::Progress { stage, fraction } => (job.stage, job.fraction) = (stage, fraction),
                Message::Preview { width, height, rgba } => {
                    job.fresh = Some(egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba,
                    ));
                }
                Message::Done(result) => done = Some(*result),
            }
        }
        if done.is_some() {
            self.job = None;
        }
        done
    }

    /// The progress window.
    pub fn window(&mut self, ctx: &egui::Context) {
        let Some(job) = self.job.as_mut() else { return };
        if let Some(image) = job.fresh.take() {
            match &mut job.texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    job.texture =
                        Some(ctx.load_texture("worldgen-preview", image, egui::TextureOptions::LINEAR))
                }
            }
        }
        egui::Window::new("Generating world")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let current = Stage::ALL.iter().position(|&s| s == job.stage).unwrap_or(0);
                for (i, stage) in Stage::ALL.iter().enumerate() {
                    let (mark, strong) = match i.cmp(&current) {
                        std::cmp::Ordering::Less => ("✔", false),
                        std::cmp::Ordering::Equal => ("▶", true),
                        std::cmp::Ordering::Greater => ("·", false),
                    };
                    let text = egui::RichText::new(format!("{mark} {}", stage.name()));
                    ui.label(if strong { text.strong() } else { text.weak() });
                }
                let overall = (current as f32 + job.fraction.clamp(0.0, 1.0)) / Stage::ALL.len() as f32;
                ui.add(
                    egui::ProgressBar::new(overall)
                        .text(format!("{:.0} s", job.started.elapsed().as_secs_f32()))
                        .desired_width(PREVIEW_WIDTH),
                );
                if let Some(texture) = &job.texture {
                    let size = texture.size_vec2();
                    let scale = (PREVIEW_WIDTH / size.x).min(420.0 / size.y);
                    ui.image((texture.id(), size * scale));
                }
            });
        // Keep repainting while waiting for news.
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

/// Draws a preview at about [`PREVIEW_WIDTH`] pixels wide.
fn preview(world: &World, registry: &Registry, view: PreviewView) -> Message {
    let width = world.topology.width().max(1) as f32;
    let layout = HexLayout { size: (PREVIEW_WIDTH / (1.5 * width)).max(0.5) };
    let mode = match view {
        PreviewView::Plates => MapMode::Plates,
        PreviewView::Elevation => MapMode::Elevation,
        PreviewView::Rainfall => MapMode::Rainfall,
    };
    let image = map_raster::render(world, registry, &RenderOptions { layout, mode, ..Default::default() });
    Message::Preview { width: image.width, height: image.height, rgba: image.rgba }
}
