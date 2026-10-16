//! Pointer tools: inspecting hexes and painting terrain.

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
}

pub const MAX_BRUSH_RADIUS: u32 = 12;

#[derive(Resource)]
pub struct ToolState {
    pub tool: Tool,
    /// What the brush paints: a biome or a relief level.
    pub brush: Paint,
    pub brush_radius: u32,
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
        let path = match *last {
            Some(prev) => world.topology.line(prev, hex).into_iter().skip(1).collect(),
            None => vec![hex],
        };
        for center in path {
            sim.submit(Command::Reshape {
                center,
                radius: self.brush_radius,
                paint: self.brush,
                edit: *edit,
            });
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
