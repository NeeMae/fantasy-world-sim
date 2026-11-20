//! Pointer tools: inspecting hexes, painting terrain and drawing rivers.

use bevy::prelude::*;
use content::BiomeId;
use sim_core::{Command, EditId, HexId, Paint, World};

use crate::planet_view::Pointer;
use crate::sim_thread::SimThread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Click a hex to inspect it.
    Inspect,
    /// Click or drag to reshape terrain.
    Paint,
    /// Drag to draw a river (or, erasing, to dry rivers up).
    River,
}

/// River sizes the river tool draws, as multiples of the river threshold.
pub const RIVER_SIZES: [(&str, f32); 4] =
    [("Stream", 1.0), ("River", 4.0), ("Big river", 16.0), ("Great river", 64.0)];

pub const MAX_BRUSH_RADIUS: u32 = 12;

#[derive(Resource)]
pub struct ToolState {
    pub tool: Tool,
    /// What the brush paints: a biome or a relief level.
    pub brush: Paint,
    pub brush_radius: u32,
    /// Index into [`RIVER_SIZES`].
    pub river_size: usize,
    /// The river tool dries rivers up instead of drawing them.
    pub river_erase: bool,
    /// Id for the next brush stroke, so each stroke is one undo step.
    next_edit: u64,
    /// The stroke being painted, and the last hex it reached.
    stroke: Option<(EditId, Option<HexId>)>,
}

impl Default for ToolState {
    fn default() -> Self {
        ToolState {
            tool: Tool::Inspect,
            brush: Paint::Biome(BiomeId(0)),
            brush_radius: 2,
            river_size: 1,
            river_erase: false,
            next_edit: 0,
            stroke: None,
        }
    }
}

impl ToolState {
    /// The hexes the current tool would act on with the pointer over `hex`.
    pub fn affected_hexes(&self, world: &World, hex: HexId) -> Vec<HexId> {
        match self.tool {
            Tool::Inspect => vec![hex],
            Tool::Paint => world.topology.within(hex, self.brush_radius),
            Tool::River if self.river_erase => world.topology.within(hex, self.brush_radius),
            Tool::River => vec![hex],
        }
    }

    /// Applies the current tool for this frame's pointer: inspect on click,
    /// paint while the button is held (each stroke one undo step, filling in
    /// hexes the pointer skipped over between frames).
    pub fn apply(&mut self, pointer: &Pointer, sim: &SimThread, world: &World, selection: &mut Selection) {
        if pointer.pressed {
            self.next_edit += 1;
            self.stroke = Some((EditId(self.next_edit), None));
        }
        if self.tool == Tool::Inspect {
            if pointer.clicked {
                selection.0 = pointer.hover;
            }
            return;
        }
        if !pointer.held {
            self.stroke = None;
            return;
        }
        let (Some((edit, last)), Some(hex)) = (self.stroke.as_mut(), pointer.hover) else { return };
        if *last == Some(hex) {
            return;
        }
        // Fill in hexes the pointer skipped over between frames.
        let line = match *last {
            Some(prev) => world.topology.line(prev, hex),
            None => vec![hex],
        };
        let edit = *edit;
        let fresh = line.iter().skip(last.is_some() as usize).copied();
        match self.tool {
            Tool::River if !self.river_erase => {
                // Each step of the stroke sends one hex's water into the next.
                let flow = RIVER_SIZES[self.river_size.min(RIVER_SIZES.len() - 1)].1;
                for pair in line.windows(2) {
                    sim.submit(Command::DrawRiver { from: pair[0], to: pair[1], flow, edit });
                }
            }
            Tool::River => {
                for center in fresh {
                    sim.submit(Command::EraseRiver { center, radius: self.brush_radius, edit });
                }
            }
            _ => {
                for center in fresh {
                    sim.submit(Command::Reshape {
                        center,
                        radius: self.brush_radius,
                        paint: self.brush,
                        edit,
                    });
                }
            }
        }
        *last = Some(hex);
    }
}

/// The hex under the pointer.
#[derive(Resource, Default)]
pub struct Hover(pub Option<HexId>);

#[derive(Resource, Default)]
pub struct Selection(pub Option<HexId>);

/// Which map view is wanted.
#[derive(Resource, Default)]
pub struct MapModeSetting(pub map_raster::MapMode);
