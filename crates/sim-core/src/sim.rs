use std::sync::Arc;

use content::Registry;

use content::{BiomeId, ReliefId};

use crate::{Chronicle, Command, EditId, EventKind, HexId, NO_DRAIN, Paint, World};

/// The editable surface of one hex: what's painted on it and the water
/// through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Surface {
    biome: BiomeId,
    relief: ReliefId,
    drain: u32,
    /// Discharge, as bits so the surface compares exactly.
    discharge: u32,
}

/// The most a drawn river may carry, in multiples of the river threshold.
pub const MAX_RIVER_FLOW: f32 = 1024.0;

/// The largest brush a single command may use, to bound the work one
/// (possibly untrusted) command can cause.
pub const MAX_RESHAPE_RADIUS: u32 = 64;

/// How many edits can be undone.
pub const UNDO_DEPTH: usize = 100;

/// A river edit in progress: hexes' surfaces as they were before it, and
/// as they are being changed to.
struct RiverEdit {
    before: Vec<(HexId, Surface)>,
    after: std::collections::HashMap<HexId, Surface>,
}

impl RiverEdit {
    fn new() -> Self {
        RiverEdit { before: Vec::new(), after: std::collections::HashMap::new() }
    }

    /// Changes a hex's surface (as already changed by this edit).
    fn set(&mut self, sim: &Simulation, hex: HexId, change: impl FnOnce(&mut Surface)) {
        let surface = self.after.entry(hex).or_insert_with(|| {
            let s = sim.surface(hex);
            self.before.push((hex, s));
            s
        });
        change(surface);
    }

    /// Applies the edit and records it for undo and in the chronicle.
    fn finish(self, sim: &mut Simulation, at: HexId, edit: EditId, tick: u64) {
        let changes: Vec<(HexId, Surface, Surface)> = self
            .before
            .into_iter()
            .map(|(hex, before)| (hex, before, self.after[&hex]))
            .filter(|(_, before, after)| before != after)
            .collect();
        if changes.is_empty() {
            return;
        }
        sim.chronicle.record(tick, EventKind::RiverRedrawn { at, hexes_changed: changes.len() as u32 });
        sim.set_surfaces(changes.iter().map(|&(hex, _, after)| (hex, after)));
        sim.journal(edit, changes);
    }
}

/// One undoable edit: every hex it changed, with its biome before and after,
/// in the order the changes were made.
#[derive(Clone, Debug)]
struct Edit {
    id: EditId,
    changes: Vec<(HexId, Surface, Surface)>,
}

/// Owns the world and advances it one tick (one month) at a time.
pub struct Simulation {
    world: World,
    registry: Arc<Registry>,
    chronicle: Chronicle,
    pending: Vec<Command>,
    /// Hexes whose terrain changed since the last [`Simulation::take_changed_hexes`].
    changed: Vec<HexId>,
    /// Edits that can be undone, oldest first, and ones undone that can be
    /// redone. Part of the state: replaying the command log rebuilds them.
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl Simulation {
    pub fn new(world: World, registry: Arc<Registry>) -> Self {
        Simulation {
            world,
            registry,
            chronicle: Chronicle::default(),
            pending: Vec::new(),
            changed: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Resumes a saved world. Its undo history isn't saved.
    pub fn from_save(data: crate::SaveData, registry: Arc<Registry>) -> Self {
        let mut sim = Simulation::new(data.world, registry);
        sim.chronicle = data.chronicle;
        sim
    }

    /// Everything needed to resume this world later. Pending commands
    /// aren't included: apply them first.
    pub fn save_data(&self, settings: String) -> crate::SaveData {
        crate::SaveData { world: self.world.clone(), chronicle: self.chronicle.clone(), settings }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn chronicle(&self) -> &Chronicle {
        &self.chronicle
    }

    /// Queues a command. It takes effect before the next tick runs, or
    /// sooner via [`Simulation::apply_pending`].
    pub fn submit(&mut self, command: Command) {
        self.pending.push(command);
    }

    /// Applies queued commands now, without advancing time.
    ///
    /// Nothing happens between ticks, so this gives exactly the same history
    /// as letting the next [`Simulation::step`] apply them; it just lets a
    /// paused world respond immediately.
    pub fn apply_pending(&mut self) {
        let tick = self.world.tick;
        for command in std::mem::take(&mut self.pending) {
            match command {
                Command::Reshape { center, radius, paint, edit } => {
                    // Commands come from outside; ignore ones that don't make sense.
                    let valid_paint = match paint {
                        Paint::Biome(b) => (b.0 as usize) < self.registry.biomes().len(),
                        Paint::Relief(r) => (r.0 as usize) < self.registry.reliefs().len(),
                    };
                    if !self.world.contains(center) || !valid_paint || radius > MAX_RESHAPE_RADIUS {
                        continue;
                    }
                    let mut changes = Vec::new();
                    for hex in self.world.topology.within(center, radius) {
                        let before = self.surface(hex);
                        let after = match paint {
                            Paint::Biome(biome) => Surface { biome, ..before },
                            Paint::Relief(relief) => Surface { relief, ..before },
                        };
                        if before != after {
                            changes.push((hex, before, after));
                        }
                    }
                    if changes.is_empty() {
                        continue;
                    }
                    self.chronicle.record(
                        tick,
                        EventKind::TerrainReshaped {
                            center,
                            radius,
                            to: paint,
                            hexes_changed: changes.len() as u32,
                        },
                    );
                    self.set_surfaces(changes.iter().map(|&(hex, _, after)| (hex, after)));
                    self.journal(edit, changes);
                }
                Command::DrawRiver { from, to, flow, edit } => {
                    let w = &self.world;
                    let valid = w.contains(from)
                        && w.contains(to)
                        && w.topology.neighbors(from).any(|n| n == to)
                        && flow.is_finite()
                        && (0.0..=MAX_RIVER_FLOW).contains(&flow)
                        && !self.is_water(from)
                        && !self.downstream(to).any(|h| h == from);
                    if !valid {
                        continue;
                    }
                    let mut edit_hexes = RiverEdit::new();
                    let t = &self.world.terrain;
                    let q = t.discharge[from.index()];
                    let q_new = q.max(flow * t.river_threshold);
                    // Take its water off the old way down, and send it the new way.
                    let old: Vec<HexId> = self.downstream(from).skip(1).collect();
                    let new: Vec<HexId> = std::iter::once(to).chain(self.downstream(to).skip(1)).collect();
                    edit_hexes.set(self, from, |s| {
                        s.drain = to.0;
                        s.discharge = q_new.to_bits();
                    });
                    for h in old {
                        edit_hexes.set(self, h, |s| {
                            s.discharge = (f32::from_bits(s.discharge) - q).max(0.0).to_bits()
                        });
                    }
                    for h in new {
                        edit_hexes
                            .set(self, h, |s| s.discharge = (f32::from_bits(s.discharge) + q_new).to_bits());
                    }
                    edit_hexes.finish(self, from, edit, tick);
                }
                Command::EraseRiver { center, radius, edit } => {
                    if !self.world.contains(center) || radius > MAX_RESHAPE_RADIUS {
                        continue;
                    }
                    let mut edit_hexes = RiverEdit::new();
                    let threshold = self.world.terrain.river_threshold;
                    let mut rivers: Vec<HexId> = self
                        .world
                        .topology
                        .within(center, radius)
                        .into_iter()
                        .filter(|h| {
                            !self.is_water(*h) && self.world.terrain.discharge[h.index()] >= threshold
                        })
                        .collect();
                    // Upstream first (a river only grows downstream), so water
                    // already taken away above isn't taken twice below.
                    rivers.sort_by(|a, b| {
                        let d = &self.world.terrain.discharge;
                        d[a.index()].total_cmp(&d[b.index()]).then(a.0.cmp(&b.0))
                    });
                    for h in rivers {
                        let q = self.world.terrain.discharge[h.index()];
                        if q < threshold {
                            continue;
                        }
                        for d in self.downstream(h).collect::<Vec<_>>() {
                            edit_hexes.set(self, d, |s| {
                                s.discharge = (f32::from_bits(s.discharge) - q).max(0.0).to_bits()
                            });
                        }
                    }
                    edit_hexes.finish(self, center, edit, tick);
                }
                Command::Undo => {
                    if let Some(edit) = self.undo.pop() {
                        self.set_surfaces(edit.changes.iter().rev().map(|&(hex, before, _)| (hex, before)));
                        self.redo.push(edit);
                    }
                }
                Command::Redo => {
                    if let Some(edit) = self.redo.pop() {
                        self.set_surfaces(edit.changes.iter().map(|&(hex, _, after)| (hex, after)));
                        self.undo.push(edit);
                    }
                }
            }
        }
    }

    /// How many edits can currently be undone and redone.
    pub fn undo_redo_depth(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    fn surface(&self, hex: HexId) -> Surface {
        let t = &self.world.terrain;
        let i = hex.index();
        Surface {
            biome: t.biome[i],
            relief: t.relief[i],
            drain: t.drain[i],
            discharge: t.discharge[i].to_bits(),
        }
    }

    fn set_surfaces(&mut self, changes: impl Iterator<Item = (HexId, Surface)>) {
        for (hex, s) in changes {
            let t = &mut self.world.terrain;
            let i = hex.index();
            let rerouted = t.drain[i] != s.drain || t.discharge[i].to_bits() != s.discharge;
            // A river is drawn towards the hex it drains into (and that
            // hex's drawing shows where rivers arrive), so both redraw.
            for d in [t.drain[i], s.drain] {
                if rerouted && d != NO_DRAIN {
                    self.changed.push(HexId(d));
                }
            }
            t.biome[i] = s.biome;
            t.relief[i] = s.relief;
            t.drain[i] = s.drain;
            t.discharge[i] = f32::from_bits(s.discharge);
            self.changed.push(hex);
        }
        self.world.terrain_revision += 1;
    }

    fn is_water(&self, hex: HexId) -> bool {
        self.registry.biome(self.world.terrain.biome[hex.index()]).water
    }

    /// `start` and the hexes its water flows through after it, to the sea
    /// (or wherever it ends). Stops if the drainage ever loops.
    fn downstream(&self, start: HexId) -> impl Iterator<Item = HexId> + '_ {
        let drain = &self.world.terrain.drain;
        let mut next = Some(start);
        let mut steps = 0;
        std::iter::from_fn(move || {
            let here = next?;
            steps += 1;
            let d = drain[here.index()];
            next = (d != NO_DRAIN && steps <= drain.len()).then_some(HexId(d));
            Some(here)
        })
    }

    /// Records changes under `edit`, merging into the latest edit if it has
    /// the same id. Any new edit makes earlier undone ones unrecoverable.
    fn journal(&mut self, edit: EditId, changes: Vec<(HexId, Surface, Surface)>) {
        self.redo.clear();
        match self.undo.last_mut() {
            Some(last) if last.id == edit => last.changes.extend(changes),
            _ => {
                self.undo.push(Edit { id: edit, changes });
                if self.undo.len() > UNDO_DEPTH {
                    self.undo.remove(0);
                }
            }
        }
    }

    /// Advances the world by one tick.
    ///
    /// Phases run in a fixed order. Later phases (population, economy, AI,
    /// war, stability, events) slot in between commands and the clock.
    pub fn step(&mut self) {
        self.apply_pending();
        self.world.tick += 1;
    }

    pub fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    /// Hexes whose terrain changed since the last call, so views can redraw
    /// just those. May contain duplicates.
    pub fn take_changed_hexes(&mut self) -> Vec<HexId> {
        std::mem::take(&mut self.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Terrain, Topology, Wrap};

    /// All land, so rivers can be drawn anywhere; returns three hexes in a row.
    fn land_sim() -> (Simulation, [HexId; 4]) {
        let mut s = sim(1);
        s.world.terrain.biome.iter_mut().for_each(|b| *b = BiomeId(1));
        let t = &s.world.topology;
        let a = HexId(9);
        let b = t.neighbors(a).next().unwrap();
        let c = t.neighbors(b).find(|&n| n != a && !t.neighbors(a).any(|m| m == n)).unwrap();
        let d = t.neighbors(a).find(|&n| n != b && n != c).unwrap();
        (s, [a, b, c, d])
    }

    fn flow(s: &Simulation, h: HexId) -> f32 {
        s.world.terrain.discharge[h.index()] / s.world.terrain.river_threshold
    }

    #[test]
    fn drawn_rivers_carry_their_water_down_and_can_be_redirected() {
        let (mut s, [a, b, c, d]) = land_sim();
        s.submit(Command::DrawRiver { from: a, to: b, flow: 2.0, edit: EditId(1) });
        s.submit(Command::DrawRiver { from: b, to: c, flow: 2.0, edit: EditId(1) });
        s.apply_pending();
        assert_eq!((flow(&s, a), flow(&s, b), flow(&s, c)), (2.0, 2.0, 2.0));
        assert_eq!(s.world.terrain.drain[a.index()], b.0);
        // Send the source another way: the old river below it dries up.
        s.submit(Command::DrawRiver { from: a, to: d, flow: 2.0, edit: EditId(2) });
        s.apply_pending();
        assert_eq!((flow(&s, b), flow(&s, c), flow(&s, d)), (0.0, 0.0, 2.0));
        // Undo puts the first river back; undo again removes it.
        s.submit(Command::Undo);
        s.apply_pending();
        assert_eq!((flow(&s, b), flow(&s, c), flow(&s, d)), (2.0, 2.0, 0.0));
        s.submit(Command::Undo);
        s.apply_pending();
        assert!([a, b, c, d].iter().all(|&h| flow(&s, h) == 0.0));
        assert!([a, b, c, d].iter().all(|&h| s.world.terrain.drain[h.index()] == NO_DRAIN));
    }

    #[test]
    fn rivers_cannot_loop_or_start_in_water() {
        let (mut s, [a, b, c, _]) = land_sim();
        s.submit(Command::DrawRiver { from: a, to: b, flow: 2.0, edit: EditId(1) });
        s.submit(Command::DrawRiver { from: b, to: c, flow: 2.0, edit: EditId(1) });
        s.apply_pending();
        let revision = s.world.terrain_revision;
        s.submit(Command::DrawRiver { from: b, to: a, flow: 2.0, edit: EditId(2) });
        s.submit(Command::DrawRiver { from: a, to: c, flow: 2.0, edit: EditId(2) }); // not neighbours
        s.submit(Command::DrawRiver { from: a, to: b, flow: f32::NAN, edit: EditId(2) });
        s.apply_pending();
        assert_eq!(s.world.terrain_revision, revision);
        s.world.terrain.biome[c.index()] = BiomeId(0);
        s.submit(Command::DrawRiver { from: c, to: b, flow: 2.0, edit: EditId(3) });
        s.apply_pending();
        assert_eq!(s.world.terrain_revision, revision);
    }

    #[test]
    fn erasing_dries_a_river_from_there_down() {
        let (mut s, [a, b, c, _]) = land_sim();
        s.submit(Command::DrawRiver { from: a, to: b, flow: 3.0, edit: EditId(1) });
        s.submit(Command::DrawRiver { from: b, to: c, flow: 3.0, edit: EditId(1) });
        s.submit(Command::EraseRiver { center: b, radius: 0, edit: EditId(2) });
        s.apply_pending();
        assert_eq!((flow(&s, a), flow(&s, b), flow(&s, c)), (3.0, 0.0, 0.0));
        s.submit(Command::Undo);
        s.apply_pending();
        assert_eq!((flow(&s, a), flow(&s, b), flow(&s, c)), (3.0, 3.0, 3.0));
    }

    fn sim(seed: u64) -> Simulation {
        let registry = content::load_str(r##"(biomes: [(id: "sea", water: true), (id: "land")])"##).unwrap();
        let topo = Topology::new(8, 6, Wrap::None);
        Simulation::new(World::new(seed, topo, Terrain::new(topo.len())), Arc::new(registry))
    }

    fn reshape(center: u32, radius: u32, biome: u16) -> Command {
        stroke(center, radius, biome, center as u64)
    }

    fn stroke(center: u32, radius: u32, biome: u16, edit: u64) -> Command {
        Command::Reshape {
            center: HexId(center),
            radius,
            paint: Paint::Biome(BiomeId(biome)),
            edit: EditId(edit),
        }
    }

    #[test]
    fn commands_apply_before_the_next_tick_and_are_chronicled() {
        let mut s = sim(1);
        s.submit(reshape(3, 0, 1));
        assert_eq!(s.world().terrain.biome[3], BiomeId(0), "queued, not yet applied");
        s.step();
        assert_eq!(s.world().terrain.biome[3], BiomeId(1));
        assert_eq!(s.world().terrain_revision, 1);
        assert_eq!(s.chronicle().entries().len(), 1);
        assert_eq!(s.chronicle().entries()[0].tick, 0, "applied before tick 0 ran");
        assert_eq!(s.world().tick, 1);
        assert_eq!(s.take_changed_hexes(), vec![HexId(3)]);
        assert!(s.take_changed_hexes().is_empty());
    }

    #[test]
    fn applying_early_gives_the_same_history() {
        let mut early = sim(1);
        let mut late = sim(1);
        for s in [&mut early, &mut late] {
            s.run(5);
            s.submit(reshape(20, 2, 1));
        }
        early.apply_pending();
        assert_eq!(early.world().tick, 5, "applying commands doesn't advance time");
        early.run(5);
        late.run(5);
        assert_eq!(early.world().state_hash(), late.world().state_hash());
        assert_eq!(early.chronicle().entries(), late.chronicle().entries());
    }

    #[test]
    fn brush_reshapes_an_area() {
        let mut s = sim(1);
        let center = s.world().topology.at(4, 3).unwrap();
        s.submit(Command::Reshape { center, radius: 1, paint: Paint::Biome(BiomeId(1)), edit: EditId(0) });
        s.apply_pending();
        let land = s.world().terrain.biome.iter().filter(|&&b| b == BiomeId(1)).count();
        assert_eq!(land, 7);
    }

    #[test]
    fn undo_reverts_a_whole_stroke_and_redo_reapplies_it() {
        let mut s = sim(1);
        let original = s.world().state_hash();
        // One stroke of overlapping stamps, then a second stroke.
        for center in [10, 11, 12] {
            s.submit(stroke(center, 1, 1, 7));
        }
        s.apply_pending();
        let after_first = s.world().terrain.biome.clone();
        s.submit(stroke(30, 0, 1, 8));
        s.apply_pending();
        assert_eq!(s.undo_redo_depth(), (2, 0));

        s.submit(Command::Undo);
        s.apply_pending();
        assert_eq!(s.world().terrain.biome, after_first, "second stroke undone");
        s.submit(Command::Undo);
        s.apply_pending();
        assert!(s.world().terrain.biome.iter().all(|&b| b == BiomeId(0)), "first stroke undone");
        assert_eq!(s.undo_redo_depth(), (0, 2));

        s.submit(Command::Redo);
        s.apply_pending();
        assert_eq!(s.world().terrain.biome, after_first);

        // A new edit discards what's left to redo.
        s.submit(stroke(40, 0, 1, 9));
        s.apply_pending();
        assert_eq!(s.undo_redo_depth(), (2, 0));
        for _ in 0..2 {
            s.submit(Command::Undo);
        }
        s.submit(Command::Undo); // nothing left: ignored
        s.apply_pending();
        assert!(s.world().terrain.biome.iter().all(|&b| b == BiomeId(0)));
        assert_ne!(s.world().state_hash(), original, "revision still advanced");
    }

    #[test]
    fn relief_paints_independently_of_biome_and_undoes() {
        let registry = content::load_str(
            r#"(biomes: [(id: "sea", water: true), (id: "land")], reliefs: [(id: "flat"), (id: "hills")])"#,
        )
        .unwrap();
        let topo = Topology::new(8, 6, Wrap::None);
        let mut s = Simulation::new(World::new(1, topo, Terrain::new(topo.len())), Arc::new(registry));
        s.submit(stroke(10, 0, 1, 1));
        s.submit(Command::Reshape {
            center: HexId(10),
            radius: 0,
            paint: Paint::Relief(ReliefId(1)),
            edit: EditId(2),
        });
        s.apply_pending();
        let t = &s.world().terrain;
        assert_eq!(
            (t.biome[10], t.relief[10]),
            (BiomeId(1), ReliefId(1)),
            "forested hills, not one or the other"
        );
        s.submit(Command::Undo);
        s.apply_pending();
        let t = &s.world().terrain;
        assert_eq!(
            (t.biome[10], t.relief[10]),
            (BiomeId(1), ReliefId(0)),
            "undoing the relief keeps the biome"
        );
        s.submit(Command::Reshape {
            center: HexId(10),
            radius: 0,
            paint: Paint::Relief(ReliefId(9)),
            edit: EditId(3),
        });
        s.apply_pending();
        assert_eq!(s.world().terrain.relief[10], ReliefId(0), "unknown relief ignored");
    }

    #[test]
    fn invalid_commands_are_ignored() {
        let mut s = sim(1);
        s.submit(reshape(10_000, 0, 1));
        s.submit(reshape(0, 0, 99));
        s.submit(reshape(0, MAX_RESHAPE_RADIUS + 1, 1));
        s.submit(reshape(0, 0, 0)); // already that biome: no change, no entry
        s.step();
        assert_eq!(s.world().terrain_revision, 0);
        assert!(s.chronicle().entries().is_empty());
    }

    #[test]
    fn same_inputs_same_history() {
        let run = |seed| {
            let mut s = sim(seed);
            for t in 0..120u32 {
                if t % 7 == 0 {
                    s.submit(reshape(t % 48, t % 3, (t % 2) as u16));
                }
                s.step();
            }
            s.world().state_hash()
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }
}
