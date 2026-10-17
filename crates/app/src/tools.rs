//! Pointer tools: inspecting hexes, painting terrain and drawing rivers.

use bevy::prelude::*;
use content::BiomeId;
use sim_core::{Command, EditId, HexId, Paint, World};

use crate::map_view::MapView;
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
}

/// The hex under the pointer, if the pointer is over the map (not the UI).
#[derive(Resource, Default)]
pub struct Hover(pub Option<HexId>);

#[derive(Resource, Default)]
pub struct Selection(pub Option<HexId>);

/// Which map view is wanted; the map redraws when it changes.
#[derive(Resource, Default)]
pub struct MapModeSetting(pub map_raster::MapMode);

/// Where the UI is, so map tools can ignore the pointer over it.
///
/// Written by the UI pass, which runs after the map systems; the panel
/// rectangles are therefore a frame old, but they're tested against the
/// *current* cursor so a click that arrives as the pointer reaches a panel is
/// still caught.
#[derive(Resource, Default)]
pub struct PointerOverUi {
    /// Panel rectangles, in logical window pixels.
    pub panels: Vec<Rect>,
    /// egui is busy with the pointer: over a floating window or popup, mid-drag
    /// on a widget, or a popup is open (so a click should just close it).
    pub busy: bool,
}

impl PointerOverUi {
    pub fn blocks(&self, cursor: Option<Vec2>) -> bool {
        self.busy || cursor.is_some_and(|c| self.panels.iter().any(|r| r.contains(c)))
    }
}

pub fn cursor(windows: &Query<&Window>) -> Option<Vec2> {
    windows.single().ok().and_then(Window::cursor_position)
}

pub fn update_hover(
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform)>,
    over_ui: Res<PointerOverUi>,
    sim: Res<SimThread>,
    view: Res<MapView>,
    mut hover: ResMut<Hover>,
) {
    let hex = (|| {
        let cursor = cursor(&windows)?;
        if over_ui.blocks(Some(cursor)) {
            return None;
        }
        let (camera, transform) = camera.single().ok()?;
        let pos = camera.viewport_to_world_2d(transform, cursor).ok()?;
        view.hex_at(&sim.snapshot().world, pos)
    })();
    if hover.0 != hex {
        hover.0 = hex;
    }
}

/// Applies the current tool on left click (and, for painting, drag).
pub fn use_tool(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    over_ui: Res<PointerOverUi>,
    hover: Res<Hover>,
    sim: Res<SimThread>,
    mut tools: ResMut<ToolState>,
    mut selection: ResMut<Selection>,
    // A stroke only counts if it started on the map, and only paints each
    // hex once as the pointer moves.
    mut stroke: Local<Option<(EditId, Option<HexId>)>>,
) {
    if buttons.just_pressed(MouseButton::Left) && !over_ui.blocks(cursor(&windows)) {
        tools.next_edit += 1;
        *stroke = Some((EditId(tools.next_edit), None));
        if tools.tool == Tool::Inspect {
            selection.0 = hover.0;
        }
    }
    // A quick flick can press, move and release within one frame; the
    // release frame still paints up to where the pointer ended.
    let released = buttons.just_released(MouseButton::Left);
    if !buttons.pressed(MouseButton::Left) && !released {
        *stroke = None;
        return;
    }
    let (Some((edit, last)), Some(hex)) = (stroke.as_mut(), hover.0) else { return };
    if tools.tool == Tool::Inspect || *last == Some(hex) {
        return;
    }
    // Fill in hexes the pointer skipped over between frames.
    let line = match *last {
        Some(prev) => sim.snapshot().world.topology.line(prev, hex),
        None => vec![hex],
    };
    let edit = *edit;
    match tools.tool {
        Tool::River if !tools.river_erase => {
            // Each step of the stroke sends one hex's water into the next.
            let flow = RIVER_SIZES[tools.river_size.min(RIVER_SIZES.len() - 1)].1;
            for pair in line.windows(2) {
                sim.submit(Command::DrawRiver { from: pair[0], to: pair[1], flow, edit });
            }
        }
        Tool::River => {
            for &center in line.iter().skip(last.is_some() as usize) {
                sim.submit(Command::EraseRiver { center, radius: tools.brush_radius, edit });
            }
        }
        _ => {
            for &center in line.iter().skip(last.is_some() as usize) {
                sim.submit(Command::Reshape { center, radius: tools.brush_radius, paint: tools.brush, edit });
            }
        }
    }
    *last = Some(hex);
    if released {
        *stroke = None;
    }
}
