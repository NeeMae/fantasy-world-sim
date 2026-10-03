use std::sync::Arc;

use content::Registry;

use crate::{Chronicle, Command, EventKind, HexId, World};

/// The largest brush a single command may use, to bound the work one
/// (possibly untrusted) command can cause.
pub const MAX_RESHAPE_RADIUS: u32 = 64;

/// Owns the world and advances it one tick (one month) at a time.
pub struct Simulation {
    world: World,
    registry: Arc<Registry>,
    chronicle: Chronicle,
    pending: Vec<Command>,
    /// Hexes whose terrain changed since the last [`Simulation::take_changed_hexes`].
    changed: Vec<HexId>,
}

impl Simulation {
    pub fn new(world: World, registry: Arc<Registry>) -> Self {
        Simulation {
            world,
            registry,
            chronicle: Chronicle::default(),
            pending: Vec::new(),
            changed: Vec::new(),
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
                Command::Reshape { center, radius, biome } => {
                    // Commands come from outside; ignore ones that don't make sense.
                    if !self.world.contains(center)
                        || biome.0 as usize >= self.registry.biomes().len()
                        || radius > MAX_RESHAPE_RADIUS
                    {
                        continue;
                    }
                    let mut hexes_changed = 0;
                    for hex in self.world.topology.within(center, radius) {
                        let slot = &mut self.world.terrain.biome[hex.index()];
                        if *slot != biome {
                            *slot = biome;
                            self.changed.push(hex);
                            hexes_changed += 1;
                        }
                    }
                    if hexes_changed > 0 {
                        self.world.terrain_revision += 1;
                        self.chronicle.record(
                            tick,
                            EventKind::TerrainReshaped { center, radius, to: biome, hexes_changed },
                        );
                    }
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
        Command::Reshape { center: HexId(center), radius, biome: BiomeId(biome) }
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
        s.submit(Command::Reshape { center, radius: 1, biome: BiomeId(1) });
        s.apply_pending();
        let land = s.world().terrain.biome.iter().filter(|&&b| b == BiomeId(1)).count();
        assert_eq!(land, 7);
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
