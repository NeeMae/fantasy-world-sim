//! Saving and loading worlds.
//!
//! A save file is a short header followed by the world and its history,
//! encoded with postcard and deflate-compressed. Biomes and reliefs are
//! stored by their def ids rather than their numbers, so a save still loads
//! after content packs add, remove or reorder defs (as long as every def the
//! world uses still exists).

use std::io::{Read, Write};
use std::path::Path;

use content::{BiomeId, Registry, ReliefId};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use serde::{Deserialize, Serialize};

use crate::{Chronicle, World};

const MAGIC: &[u8; 8] = b"FWSSAVE\0";
/// Bumped whenever the saved layout changes incompatibly.
pub const SAVE_VERSION: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("not a world save")]
    NotASave,
    #[error("saved by a newer or incompatible version (format {0}, expected {SAVE_VERSION})")]
    Version(u32),
    #[error("corrupt save: {0}")]
    Corrupt(#[from] postcard::Error),
    #[error("the save uses {kind} {id:?}, which the loaded content doesn't define")]
    MissingDef { kind: &'static str, id: String },
    #[error("corrupt save: terrain doesn't match the map size")]
    Mismatch,
}

/// Everything a save holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveData {
    pub world: World,
    pub chronicle: Chronicle,
    /// Free-form settings from whatever made the save (the app keeps its
    /// world-generation form here), so they can be restored too.
    pub settings: String,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    biomes: Vec<String>,
    reliefs: Vec<String>,
    data: SaveData,
}

/// Writes a save, replacing any file already at `path`. Writes to a
/// temporary file first, so a failed save never destroys an older one.
pub fn save(path: &Path, data: &SaveData, registry: &Registry) -> Result<(), SaveError> {
    let stored = Stored {
        biomes: registry.biomes().iter().map(|b| b.id.clone()).collect(),
        reliefs: registry.reliefs().iter().map(|r| r.id.clone()).collect(),
        data: data.clone(),
    };
    let bytes = postcard::to_stdvec(&stored)?;
    let mut out = Vec::with_capacity(bytes.len() / 3);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SAVE_VERSION.to_le_bytes());
    let mut encoder = DeflateEncoder::new(out, Compression::default());
    encoder.write_all(&bytes)?;
    let out = encoder.finish()?;

    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Reads a save, renumbering its biomes and reliefs for `registry`.
pub fn load(path: &Path, registry: &Registry) -> Result<SaveData, SaveError> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err(SaveError::NotASave);
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes"));
    let mut raw = Vec::new();
    DeflateDecoder::new(&bytes[12..]).read_to_end(&mut raw)?;
    let stored: Stored = match version {
        SAVE_VERSION => postcard::from_bytes(&raw)?,
        2 => postcard::from_bytes::<v2::Stored>(&raw)?.into(),
        _ => return Err(SaveError::Version(version)),
    };
    let mut data = stored.data;

    let t = &mut data.world.terrain;
    let n = data.world.topology.len();
    if [
        t.elevation.len(),
        t.moisture.len(),
        t.temperature.len(),
        t.biome.len(),
        t.relief.len(),
        t.drain.len(),
    ]
    .iter()
    .any(|&len| len != n)
    {
        return Err(SaveError::Mismatch);
    }
    let biomes: Vec<Option<u16>> =
        stored.biomes.iter().map(|id| registry.biome_id(id).map(|b| b.0)).collect();
    let reliefs: Vec<Option<u8>> =
        stored.reliefs.iter().map(|id| registry.relief_id(id).map(|r| r.0)).collect();
    for b in &mut t.biome {
        *b = BiomeId(remap(&biomes, &stored.biomes, b.0 as usize, "biome")?);
    }
    for r in &mut t.relief {
        *r = ReliefId(remap(&reliefs, &stored.reliefs, r.0 as usize, "relief")?);
    }
    Ok(data)
}

/// The registry's number for saved def number `i`. Defs the save lists but
/// the world doesn't use needn't exist any more.
fn remap<T: Copy>(map: &[Option<T>], ids: &[String], i: usize, kind: &'static str) -> Result<T, SaveError> {
    match map.get(i) {
        Some(Some(v)) => Ok(*v),
        Some(None) => Err(SaveError::MissingDef { kind, id: ids[i].clone() }),
        None => Err(SaveError::Mismatch),
    }
}

/// Saves from v0.2.0 (format 2), before the river threshold was a setting.
/// They load with the threshold v0.2.0 used.
mod v2 {
    use serde::{Deserialize, Serialize};

    use crate::{Chronicle, Geology, Topology, WorldRules};
    use content::{BiomeId, ReliefId};

    #[derive(Serialize, Deserialize)]
    pub struct Terrain {
        pub elevation: Vec<f32>,
        pub moisture: Vec<f32>,
        pub temperature: Vec<f32>,
        pub ruggedness: Vec<f32>,
        pub massif: Vec<f32>,
        pub biome: Vec<BiomeId>,
        pub relief: Vec<ReliefId>,
        pub drain: Vec<u32>,
        pub discharge: Vec<f32>,
        pub current: Vec<f32>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct World {
        pub seed: u64,
        pub tick: u64,
        pub topology: Topology,
        pub terrain: Terrain,
        pub geology: Geology,
        pub rules: WorldRules,
        pub terrain_revision: u64,
    }

    #[derive(Serialize, Deserialize)]
    pub struct SaveData {
        pub world: World,
        pub chronicle: Chronicle,
        pub settings: String,
    }

    #[derive(Serialize, Deserialize)]
    pub struct Stored {
        pub biomes: Vec<String>,
        pub reliefs: Vec<String>,
        pub data: SaveData,
    }

    impl From<Stored> for super::Stored {
        fn from(old: Stored) -> Self {
            let w = old.data.world;
            let t = w.terrain;
            let terrain = crate::Terrain {
                elevation: t.elevation,
                moisture: t.moisture,
                temperature: t.temperature,
                ruggedness: t.ruggedness,
                massif: t.massif,
                biome: t.biome,
                relief: t.relief,
                drain: t.drain,
                discharge: t.discharge,
                river_threshold: crate::RIVER_DISCHARGE,
                current: t.current,
            };
            let world = crate::World {
                seed: w.seed,
                tick: w.tick,
                topology: w.topology,
                terrain,
                geology: w.geology,
                rules: w.rules,
                terrain_revision: w.terrain_revision,
            };
            super::Stored {
                biomes: old.biomes,
                reliefs: old.reliefs,
                data: super::SaveData { world, chronicle: old.data.chronicle, settings: old.data.settings },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{Command, EditId, HexId, Paint, Simulation, Terrain, Topology, Wrap};

    fn registry(biomes: &str) -> Registry {
        let dir = std::env::temp_dir().join(format!("fws-save-{}-{}", std::process::id(), biomes.len()));
        std::fs::create_dir_all(dir.join("defs")).unwrap();
        std::fs::write(dir.join("pack.ron"), "(id: \"test\", name: \"Test\")").unwrap();
        std::fs::write(dir.join("defs/biomes.ron"), format!("(biomes: {biomes})")).unwrap();
        content::load_packs(&[dir]).unwrap()
    }

    #[test]
    fn a_saved_world_loads_back_identically_even_if_defs_are_reordered() {
        let first = registry(r#"[(id: "sea", water: true), (id: "grass"), (id: "sand")]"#);
        let topo = Topology::new(8, 6, Wrap::None);
        let mut terrain = Terrain::new(topo.len());
        terrain.biome.fill(first.biome_id("grass").unwrap());
        let mut sim = Simulation::new(World::new(7, topo, terrain), Arc::new(first.clone()));
        let sand = first.biome_id("sand").unwrap();
        sim.submit(Command::Reshape {
            center: HexId(10),
            radius: 1,
            paint: Paint::Biome(sand),
            edit: EditId(1),
        });
        sim.run(5);
        let path = std::env::temp_dir().join(format!("fws-save-test-{}.world", std::process::id()));
        save(&path, &sim.save_data("settings".into()), &first).unwrap();

        let second = registry(r#"[(id: "sand"), (id: "unused"), (id: "grass"), (id: "sea", water: true)]"#);
        let loaded = load(&path, &second).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(loaded.settings, "settings");
        // Saving and loading doesn't change history: carry on in both and
        // the states agree.
        let mut resumed = Simulation::from_save(loaded.clone(), Arc::new(second.clone()));
        sim.run(7);
        resumed.run(7);
        assert_eq!(sim.world().tick, resumed.world().tick);
        assert_eq!(sim.world().terrain_revision, resumed.world().terrain_revision);
        assert_eq!(loaded.world.tick, 5);
        assert_eq!(loaded.chronicle.entries(), sim.chronicle().entries());
        for (a, b) in sim.world().terrain.biome.iter().zip(&loaded.world.terrain.biome) {
            assert_eq!(first.biome(*a).id, second.biome(*b).id);
        }
    }

    #[test]
    fn v0_2_saves_still_load() {
        let registry = registry(r#"[(id: "sea", water: true), (id: "grass")]"#);
        let topo = Topology::new(4, 3, Wrap::None);
        let n = topo.len();
        let old = v2::Stored {
            biomes: vec!["sea".into(), "grass".into()],
            reliefs: registry.reliefs().iter().map(|r| r.id.clone()).collect(),
            data: v2::SaveData {
                world: v2::World {
                    seed: 9,
                    tick: 30,
                    topology: topo,
                    terrain: v2::Terrain {
                        elevation: vec![0.5; n],
                        moisture: vec![0.5; n],
                        temperature: vec![0.5; n],
                        ruggedness: vec![0.0; n],
                        massif: vec![0.0; n],
                        biome: vec![BiomeId(1); n],
                        relief: vec![ReliefId(0); n],
                        drain: vec![crate::NO_DRAIN; n],
                        discharge: vec![crate::RIVER_DISCHARGE * 2.0; n],
                        current: vec![0.0; n],
                    },
                    geology: crate::Geology::default(),
                    rules: crate::WorldRules::default(),
                    terrain_revision: 0,
                },
                chronicle: Chronicle::default(),
                settings: "old".into(),
            },
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        let mut encoder = DeflateEncoder::new(bytes, Compression::default());
        encoder.write_all(&postcard::to_stdvec(&old).unwrap()).unwrap();
        let path = std::env::temp_dir().join(format!("fws-v2-{}.world", std::process::id()));
        std::fs::write(&path, encoder.finish().unwrap()).unwrap();
        let loaded = load(&path, &registry).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(loaded.world.tick, 30);
        assert_eq!(loaded.settings, "old");
        assert_eq!(loaded.world.terrain.river_threshold, crate::RIVER_DISCHARGE);
        assert_eq!(loaded.world.terrain.river(0), 2.0, "rivers as v0.2.0 drew them");
    }

    #[test]
    fn junk_is_not_a_save() {
        let path = std::env::temp_dir().join(format!("fws-junk-{}.world", std::process::id()));
        std::fs::write(&path, b"hello").unwrap();
        let r = load(&path, &registry(r#"[(id: "sea", water: true)]"#));
        std::fs::remove_file(&path).ok();
        assert!(matches!(r, Err(SaveError::NotASave)));
    }
}
