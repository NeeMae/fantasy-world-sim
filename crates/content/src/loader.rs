use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ron::{Map, Value};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Registry;

/// Field names with loader meaning; they are stripped before typed parsing.
const ID: &str = "id";
const PARENT: &str = "parent";
const ABSTRACT: &str = "abstract";

/// Def kinds a def file may contain, in the order they're built.
const KINDS: &[&str] = &["biomes", "reliefs", "races", "cultures"];

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Parse { path: PathBuf, source: Box<ron::error::SpannedError> },
    #[error("{origin}: {message}")]
    Invalid { origin: String, message: String },
}

impl ContentError {
    fn invalid(origin: impl Into<String>, message: impl Into<String>) -> Self {
        ContentError::Invalid { origin: origin.into(), message: message.into() }
    }
}

/// `pack.ron`, at the root of every content pack.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
}

/// One def as written, before inheritance is resolved.
struct RawDef {
    fields: Map,
    /// Where the def was (last) defined, for error messages.
    origin: String,
}

/// All defs of one kind, in first-definition order.
#[derive(Default)]
struct RawKind {
    order: Vec<String>,
    defs: HashMap<String, RawDef>,
}

/// Where the base pack is: `packs/base` in the working directory (running
/// from a source checkout) or next to the executable (a release download,
/// launched by double-clicking from anywhere).
pub fn default_base_pack() -> PathBuf {
    let relative = Path::new("packs").join("base");
    if relative.join("pack.ron").exists() {
        return relative;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&relative)))
        .filter(|p| p.join("pack.ron").exists())
        .unwrap_or(relative)
}

/// Loads `packs` in order (later packs override earlier ones) into a [`Registry`].
pub fn load_packs<P: AsRef<Path>>(packs: &[P]) -> Result<Registry, ContentError> {
    let mut kinds: HashMap<&'static str, RawKind> = HashMap::new();
    let mut manifests = Vec::new();

    for pack in packs {
        let pack = pack.as_ref();
        let manifest_path = pack.join("pack.ron");
        let manifest: PackManifest = parse_file(&manifest_path)?;

        for file in def_files(&pack.join("defs"))? {
            let value: Value = parse_file(&file)?;
            add_def_file(&mut kinds, &file, value)?;
        }
        manifests.push(manifest);
    }

    build_registry(manifests, kinds)
}

/// Parses one def file's worth of RON as if it were the only pack.
/// Mostly useful for tests and tools.
pub fn load_str(source: &str) -> Result<Registry, ContentError> {
    let mut kinds = HashMap::new();
    let value: Value = ron::from_str(source)
        .map_err(|source| ContentError::Parse { path: "<string>".into(), source: Box::new(source) })?;
    add_def_file(&mut kinds, Path::new("<string>"), value)?;
    build_registry(Vec::new(), kinds)
}

fn build_registry(
    manifests: Vec<PackManifest>,
    mut kinds: HashMap<&'static str, RawKind>,
) -> Result<Registry, ContentError> {
    let biomes = build_kind(kinds.remove("biomes").unwrap_or_default())?;
    let reliefs = build_kind(kinds.remove("reliefs").unwrap_or_default())?;
    let races = build_kind(kinds.remove("races").unwrap_or_default())?;
    let cultures = build_kind(kinds.remove("cultures").unwrap_or_default())?;
    Registry::new(manifests, biomes, reliefs, races, cultures)
}

fn parse_file<T: DeserializeOwned>(path: &Path) -> Result<T, ContentError> {
    let text = fs::read_to_string(path).map_err(|source| ContentError::Io { path: path.into(), source })?;
    ron::from_str(&text).map_err(|source| ContentError::Parse { path: path.into(), source: Box::new(source) })
}

/// All `.ron` files below `dir`, recursively, in sorted order so loading is
/// deterministic across platforms.
fn def_files(dir: &Path) -> Result<Vec<PathBuf>, ContentError> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).map_err(|source| ContentError::Io { path: d.clone(), source })?;
        for entry in entries {
            let path = entry.map_err(|source| ContentError::Io { path: d.clone(), source })?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "ron") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn add_def_file(
    kinds: &mut HashMap<&'static str, RawKind>,
    file: &Path,
    value: Value,
) -> Result<(), ContentError> {
    let file_name = file.display().to_string();
    let Value::Map(top) = value else {
        return Err(ContentError::invalid(file_name, "a def file must be a struct like `(biomes: [...])`"));
    };
    for (key, list) in top {
        let kind_name = as_str(&key).unwrap_or_default();
        let Some(kind) = KINDS.iter().find(|k| **k == kind_name) else {
            return Err(ContentError::invalid(
                file_name,
                format!("unknown def kind {kind_name:?}; expected one of {KINDS:?}"),
            ));
        };
        let Value::Seq(list) = list else {
            return Err(ContentError::invalid(file_name, format!("`{kind}` must be a list")));
        };
        let raw = kinds.entry(kind).or_default();
        for (i, def) in list.into_iter().enumerate() {
            let origin = format!("{file_name} {kind}[{i}]");
            let Value::Map(fields) = def else {
                return Err(ContentError::invalid(origin, "a def must be a struct like `(id: \"...\")`"));
            };
            let id = fields
                .get(&Value::String(ID.into()))
                .and_then(as_str)
                .ok_or_else(|| ContentError::invalid(&origin, "missing string field `id`"))?
                .to_string();
            let origin = format!("{origin} ({id})");
            match raw.defs.get_mut(&id) {
                Some(existing) => {
                    merge(&mut existing.fields, fields);
                    existing.origin = origin;
                }
                None => {
                    raw.order.push(id.clone());
                    raw.defs.insert(id, RawDef { fields, origin });
                }
            }
        }
    }
    Ok(())
}

/// Resolves inheritance and parses every concrete def of one kind.
fn build_kind<T: DeserializeOwned>(raw: RawKind) -> Result<Vec<T>, ContentError> {
    let mut resolved: HashMap<String, Map> = HashMap::new();
    let mut out = Vec::new();
    for id in &raw.order {
        let fields = resolve(&raw, id, &mut resolved, &mut Vec::new())?;
        if fields.get(&Value::String(ABSTRACT.into())) == Some(&Value::Bool(true)) {
            continue;
        }
        let mut fields = fields.clone();
        fields.remove(&Value::String(ABSTRACT.into()));
        fields.remove(&Value::String(PARENT.into()));
        let def = Value::Map(fields)
            .into_rust::<T>()
            .map_err(|e| ContentError::invalid(&raw.defs[id].origin, e.to_string()))?;
        out.push(def);
    }
    Ok(out)
}

/// Returns `id`'s fields with its parent chain merged underneath.
fn resolve<'a>(
    raw: &RawKind,
    id: &str,
    resolved: &'a mut HashMap<String, Map>,
    visiting: &mut Vec<String>,
) -> Result<&'a Map, ContentError> {
    if !resolved.contains_key(id) {
        let def = &raw.defs[id];
        if visiting.iter().any(|v| v == id) {
            visiting.push(id.into());
            return Err(ContentError::invalid(
                &def.origin,
                format!("inheritance cycle: {}", visiting.join(" -> ")),
            ));
        }
        let mut fields = match def.fields.get(&Value::String(PARENT.into())) {
            Some(parent) => {
                let parent = as_str(parent)
                    .ok_or_else(|| ContentError::invalid(&def.origin, "`parent` must be a string id"))?;
                if !raw.defs.contains_key(parent) {
                    return Err(ContentError::invalid(&def.origin, format!("unknown parent {parent:?}")));
                }
                visiting.push(id.into());
                let mut base = resolve(raw, parent, resolved, visiting)?.clone();
                visiting.pop();
                // `abstract` describes the parent itself and is not inherited.
                base.remove(&Value::String(ABSTRACT.into()));
                base
            }
            None => Map::new(),
        };
        merge(&mut fields, def.fields.clone());
        resolved.insert(id.into(), fields);
    }
    Ok(&resolved[id])
}

/// Deep-merges `over` into `base`: nested structs merge, anything else replaces.
fn merge(base: &mut Map, over: Map) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Map(b)), Value::Map(o)) => merge(b, o),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn as_str(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherits_from_abstract_parent() {
        let reg = load_str(
            r##"(
                biomes: [
                    (id: "land", abstract: true, water: false, color: "#00ff00", priority: 3),
                    (id: "forest", parent: "land", name: "Forest", color: "#006600"),
                ],
            )"##,
        )
        .unwrap();
        assert_eq!(reg.biomes().len(), 1, "abstract defs are not instantiated");
        let forest = &reg.biomes()[0];
        assert_eq!(forest.name, "Forest");
        assert_eq!(forest.priority, 3, "inherited from parent");
        assert_eq!(forest.color, crate::Rgb(0, 0x66, 0), "child overrides parent");
    }

    #[test]
    fn redefinition_merges_over_original() {
        let reg = load_str(
            r##"(
                biomes: [
                    (id: "sea", name: "Sea", water: true, color: "#0000ff"),
                    (id: "sea", color: "#000088"),
                ],
            )"##,
        )
        .unwrap();
        let sea = &reg.biomes()[0];
        assert!(sea.water && sea.name == "Sea");
        assert_eq!(sea.color, crate::Rgb(0, 0, 0x88));
    }

    #[test]
    fn reliefs_load_and_default_to_flat() {
        let none = load_str(r#"(biomes: [(id: "sea")])"#).unwrap();
        assert_eq!(none.reliefs().len(), 1);
        assert_eq!(none.reliefs()[0].id, "flat");

        let some =
            load_str(r#"(reliefs: [(id: "flat"), (id: "hills", ruggedness: (0.3, 0.6), glyph: "hills")])"#)
                .unwrap();
        let hills = some.relief(some.relief_id("hills").unwrap());
        assert_eq!(hills.glyph, crate::Glyph::Hills);
        assert_eq!(hills.ruggedness, crate::Range(0.3, 0.6));
    }

    #[test]
    fn biomes_resolve_relief_ids_and_report_unknown_ones() {
        let reg = load_str(
            r#"(reliefs: [(id: "flat"), (id: "mountains")], biomes: [(id: "sea", water: true, elevation: (0.0, 0.6)), (id: "alpine", relief: ["mountains"])])"#,
        )
        .unwrap();
        let alpine = reg.biome(reg.biome_id("alpine").unwrap());
        assert_eq!(alpine.relief_ids, vec![reg.relief_id("mountains").unwrap()]);
        assert_eq!(reg.sea_level(), 0.6);

        let bad = load_str(r#"(biomes: [(id: "alpine", relief: ["mountain"])])"#);
        assert!(bad.unwrap_err().to_string().contains("unknown relief"));
    }

    #[test]
    fn cultures_validate_their_race_and_language() {
        let ok = load_str(
            r#"(
                races: [(id: "elf", name: "Elves", lifespan: 700.0)],
                cultures: [(id: "ilvaren", race: "elf", language: (
                    consonants: ["l", "r"], vowels: ["a", "e"], syllables: ["CV"],
                ))],
            )"#,
        )
        .unwrap();
        let culture = &ok.cultures()[0];
        assert_eq!(ok.race(culture.race_id).name, "Elves");
        assert_eq!(culture.def.title_tiers.len(), 4);

        let no_race = load_str(
            r#"(cultures: [(id: "x", race: "elf", language: (consonants: ["l"], vowels: ["a"], syllables: ["CV"]))])"#,
        );
        assert!(no_race.unwrap_err().to_string().contains("unknown race"));

        let bad_pattern = load_str(
            r#"(races: [(id: "elf")], cultures: [(id: "x", race: "elf", language: (consonants: ["l"], vowels: ["a"], syllables: ["CX"]))])"#,
        );
        assert!(bad_pattern.unwrap_err().to_string().contains("syllable"));
    }

    #[test]
    fn reports_cycles_unknown_parents_and_fields() {
        let cycle = load_str(r#"(biomes: [(id: "a", parent: "b"), (id: "b", parent: "a")])"#);
        assert!(cycle.unwrap_err().to_string().contains("cycle"));

        let missing = load_str(r#"(biomes: [(id: "a", parent: "nope")])"#);
        assert!(missing.unwrap_err().to_string().contains("unknown parent"));

        let typo = load_str(r##"(biomes: [(id: "a", colour: "#000000")])"##);
        assert!(typo.unwrap_err().to_string().contains("colour"));

        let kind = load_str(r#"(biomez: [])"#);
        assert!(kind.unwrap_err().to_string().contains("unknown def kind"));
    }
}
