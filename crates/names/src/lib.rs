//! Names from a culture's language.
//!
//! A name is a few syllables, each following one of the language's
//! patterns (`C` consonant, `V` vowel), sometimes finished with an ending
//! for people or places. Names with forbidden sequences, awkward repeated
//! letters, or silly lengths are thrown away and drawn again, so every
//! culture's names stay recognisably its own.

use content::LanguageDef;
use rand::{Rng, RngExt};

/// What a name is for, which decides the endings it can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Person,
    Place,
}

/// Shortest and longest acceptable names, in characters.
const LENGTH: std::ops::RangeInclusive<usize> = 3..=13;
/// Draws before settling for the last attempt.
const ATTEMPTS: usize = 64;

/// A name in `language`, drawn from `rng`. The same random stream always
/// gives the same name.
pub fn generate<R: Rng + ?Sized>(language: &LanguageDef, kind: NameKind, rng: &mut R) -> String {
    let mut name = String::new();
    for _ in 0..ATTEMPTS {
        name = draw(language, kind, rng);
        if acceptable(language, &name) {
            break;
        }
    }
    capitalise(&name)
}

fn draw<R: Rng + ?Sized>(language: &LanguageDef, kind: NameKind, rng: &mut R) -> String {
    let pick = |list: &[String], rng: &mut R| list[rng.random_range(0..list.len())].clone();
    let syllables = rng.random_range(language.min_syllables..=language.max_syllables);
    let endings = match kind {
        NameKind::Person => &language.person_endings,
        NameKind::Place => &language.place_endings,
    };
    let ending =
        (!endings.is_empty() && rng.random::<f32>() < language.ending_chance).then(|| pick(endings, rng));
    // An ending stands in for the last syllable, so names don't run long.
    let count = if ending.is_some() { syllables.saturating_sub(1).max(1) } else { syllables };

    let mut name = String::new();
    for _ in 0..count {
        for slot in pick(&language.syllables, rng).chars() {
            let list = if slot == 'C' { &language.consonants } else { &language.vowels };
            name.push_str(&pick(list, rng));
        }
    }
    if let Some(ending) = ending {
        name.push_str(&ending);
    }
    name.to_lowercase()
}

fn acceptable(language: &LanguageDef, name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    LENGTH.contains(&chars.len())
        && !language.forbidden.iter().any(|f| !f.is_empty() && name.contains(&f.to_lowercase()))
        // Three of the same letter in a row reads as a typo, and three
        // vowels in a row as mush, in any language.
        && !chars.windows(3).any(|w| w[0] == w[1] && w[1] == w[2])
        && !chars.windows(3).any(|w| w.iter().all(|c| "aeiou".contains(*c)))
}

fn capitalise(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn elvish() -> LanguageDef {
        LanguageDef {
            consonants: ["l", "r", "n", "th", "v", "s"].map(String::from).to_vec(),
            vowels: ["a", "e", "i", "ae"].map(String::from).to_vec(),
            syllables: ["CV", "CVC", "V"].map(String::from).to_vec(),
            min_syllables: 2,
            max_syllables: 3,
            forbidden: vec!["thth".into(), "aeae".into()],
            person_endings: vec!["iel".into(), "wen".into()],
            place_endings: vec!["ion".into()],
            ending_chance: 0.5,
        }
    }

    #[test]
    fn deterministic() {
        let a = generate(&elvish(), NameKind::Place, &mut ChaCha8Rng::seed_from_u64(9));
        let b = generate(&elvish(), NameKind::Place, &mut ChaCha8Rng::seed_from_u64(9));
        assert_eq!(a, b);
    }

    #[test]
    fn names_follow_the_language() {
        let lang = elvish();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let letters: String = "lrnthvsaeiowion".into();
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..500 {
            let name = generate(&lang, NameKind::Person, &mut rng);
            assert!(LENGTH.contains(&name.chars().count()), "{name}");
            assert!(name.chars().next().unwrap().is_uppercase(), "{name}");
            let lower = name.to_lowercase();
            assert!(!lower.contains("thth") && !lower.contains("aeae"), "{name}");
            let vowel_run =
                lower.chars().collect::<Vec<_>>().windows(3).any(|w| w.iter().all(|c| "aeiou".contains(*c)));
            assert!(!vowel_run, "no three vowels in a row: {name}");
            assert!(lower.chars().all(|c| letters.contains(c)), "only the language's sounds: {name}");
            distinct.insert(name);
        }
        assert!(distinct.len() > 200, "plenty of variety ({})", distinct.len());
    }

    #[test]
    fn place_and_person_endings_differ() {
        let lang = elvish();
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        let places: Vec<_> = (0..200).map(|_| generate(&lang, NameKind::Place, &mut rng)).collect();
        assert!(places.iter().any(|n| n.ends_with("ion")));
        assert!(!places.iter().any(|n| n.ends_with("wen")));
    }
}
