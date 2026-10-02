use std::sync::Arc;

use content::Registry;

use crate::{Chronicle, Command, EventKind, World};

/// Owns the world and advances it one tick (one month) at a time.
pub struct Simulation {
    world: World,
    registry: Arc<Registry>,
    chronicle: Chronicle,
    pending: Vec<Command>,
}

impl Simulation {
    pub fn new(world: World, registry: Arc<Registry>) -> Self {
        Simulation { world, registry, chronicle: Chronicle::default(), pending: Vec::new() }
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

    /// Queues a command for the start of the next tick.
    pub fn submit(&mut self, command: Command) {
        self.pending.push(command);
    }

    /// Advances the world by one tick.
    ///
    /// Phases run in a fixed order. Later phases (population, economy, AI,
    /// war, stability, events) slot in between commands and the clock.
    pub fn step(&mut self) {
        self.apply_commands();
        self.world.tick += 1;
    }

    pub fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    fn apply_commands(&mut self) {
        let tick = self.world.tick;
        for command in std::mem::take(&mut self.pending) {
            match command {
                Command::SetBiome { hex, biome } => {
                    // Commands come from outside; ignore ones that don't make sense.
                    if !self.world.contains(hex) || biome.0 as usize >= self.registry.biomes().len() {
                        continue;
                    }
                    let slot = &mut self.world.terrain.biome[hex.index()];
                    if *slot != biome {
                        let from = std::mem::replace(slot, biome);
                        self.world.terrain_revision += 1;
                        self.chronicle.record(tick, EventKind::TerrainReshaped { hex, from, to: biome });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HexId, Terrain, Topology, Wrap};
    use content::BiomeId;

    fn sim(seed: u64) -> Simulation {
        let registry = content::load_str(r##"(biomes: [(id: "sea", water: true), (id: "land")])"##).unwrap();
        let topo = Topology::new(8, 6, Wrap::None);
        Simulation::new(World::new(seed, topo, Terrain::new(topo.len())), Arc::new(registry))
    }

    #[test]
    fn commands_apply_on_next_tick_and_are_chronicled() {
        let mut s = sim(1);
        s.submit(Command::SetBiome { hex: HexId(3), biome: BiomeId(1) });
        assert_eq!(s.world().terrain.biome[3], BiomeId(0), "not applied until the tick runs");
        s.step();
        assert_eq!(s.world().terrain.biome[3], BiomeId(1));
        assert_eq!(s.world().terrain_revision, 1);
        assert_eq!(s.chronicle().entries().len(), 1);
        assert_eq!(s.world().tick, 1);
    }

    #[test]
    fn invalid_commands_are_ignored() {
        let mut s = sim(1);
        s.submit(Command::SetBiome { hex: HexId(10_000), biome: BiomeId(1) });
        s.submit(Command::SetBiome { hex: HexId(0), biome: BiomeId(99) });
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
                    s.submit(Command::SetBiome { hex: HexId(t % 48), biome: BiomeId((t % 2) as u16) });
                }
                s.step();
            }
            s.world().state_hash()
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }
}
