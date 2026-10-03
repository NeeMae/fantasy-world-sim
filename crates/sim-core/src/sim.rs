use std::sync::Arc;

use content::Registry;

use content::BiomeId;

use crate::{Chronicle, Command, EditId, EventKind, HexId, World};

/// The largest brush a single command may use, to bound the work one
/// (possibly untrusted) command can cause.
pub const MAX_RESHAPE_RADIUS: u32 = 64;

/// How many edits can be undone.
pub const UNDO_DEPTH: usize = 100;

/// One undoable edit: every hex it changed, with its biome before and after,
/// in the order the changes were made.
#[derive(Clone, Debug)]
struct Edit {
    id: EditId,
    changes: Vec<(HexId, BiomeId, BiomeId)>,
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
                Command::Reshape { center, radius, biome, edit } => {
                    // Commands come from outside; ignore ones that don't make sense.
                    if !self.world.contains(center)
                        || biome.0 as usize >= self.registry.biomes().len()
                        || radius > MAX_RESHAPE_RADIUS
                    {
                        continue;
                    }
                    let mut changes = Vec::new();
                    for hex in self.world.topology.within(center, radius) {
                        let before = self.world.terrain.biome[hex.index()];
                        if before != biome {
                            changes.push((hex, before, biome));
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
                            to: biome,
                            hexes_changed: changes.len() as u32,
                        },
                    );
                    self.set_biomes(changes.iter().map(|&(hex, _, after)| (hex, after)));
                    self.journal(edit, changes);
                }
                Command::Undo => {
                    if let Some(edit) = self.undo.pop() {
                        self.set_biomes(edit.changes.iter().rev().map(|&(hex, before, _)| (hex, before)));
                        self.redo.push(edit);
                    }
                }
                Command::Redo => {
                    if let Some(edit) = self.redo.pop() {
                        self.set_biomes(edit.changes.iter().map(|&(hex, _, after)| (hex, after)));
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

    fn set_biomes(&mut self, changes: impl Iterator<Item = (HexId, BiomeId)>) {
        for (hex, biome) in changes {
            self.world.terrain.biome[hex.index()] = biome;
            self.changed.push(hex);
        }
        self.world.terrain_revision += 1;
    }

    /// Records changes under `edit`, merging into the latest edit if it has
    /// the same id. Any new edit makes earlier undone ones unrecoverable.
    fn journal(&mut self, edit: EditId, changes: Vec<(HexId, BiomeId, BiomeId)>) {
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
    use content::BiomeId;

    fn sim(seed: u64) -> Simulation {
        let registry = content::load_str(r##"(biomes: [(id: "sea", water: true), (id: "land")])"##).unwrap();
        let topo = Topology::new(8, 6, Wrap::None);
        Simulation::new(World::new(seed, topo, Terrain::new(topo.len())), Arc::new(registry))
    }

    fn reshape(center: u32, radius: u32, biome: u16) -> Command {
        stroke(center, radius, biome, center as u64)
    }

    fn stroke(center: u32, radius: u32, biome: u16, edit: u64) -> Command {
        Command::Reshape { center: HexId(center), radius, biome: BiomeId(biome), edit: EditId(edit) }
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
        s.submit(Command::Reshape { center, radius: 1, biome: BiomeId(1), edit: EditId(0) });
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
