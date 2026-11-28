use crate::app::{App, lock};
use crate::engine;
use crate::util::{log, now_ms};
use nixa_protocol::TermId;
use std::collections::{HashMap, HashSet};

pub const NAME_LABEL: &str = "nixa.name";
pub const VOICE_LABEL: &str = "nixa.voice";

pub const WOMEN: [&str; 32] = [
    "Maya", "Juno", "Nova", "Kira", "Zara", "Luna", "Iris", "Ada", "Nina", "Mira", "Tara", "Esha", "Rosa", "Lila",
    "Sana", "Pia", "Yara", "Cleo", "Aria", "Dina", "Ruby", "Gita", "Vera", "Ella", "Lena", "Skye", "Asha", "Ivy",
    "Priya", "Tess", "Wren", "Mina",
];

pub const MEN: [&str; 32] = [
    "Arlo", "Ravi", "Otis", "Luca", "Theo", "Dev", "Felix", "Kai", "Leo", "Omar", "Finn", "Hugo", "Ivan", "Noah",
    "Ezra", "Jack", "Neel", "Max", "Ben", "Sami", "Alex", "Toby", "Rohan", "Bodhi", "Arun", "Nico", "Remy", "Zane",
    "Owen", "Kabir", "Jude", "Rex",
];

pub const WOMEN_VOICES: [&str; 13] = [
    "Zoey", "Juniper", "Everly", "Harper", "Hadley", "Delia", "Maeve", "Freya", "Pippa", "Elodie-Rose", "Imogen",
    "Lottie", "Bryony",
];

pub const MEN_VOICES: [&str; 12] = [
    "Easton", "Micah", "Trevor", "Russell", "Preston", "Miles", "Alfie", "Hugh", "Edwin", "Spencer", "Toby", "Teddy",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Woman,
    Man,
}

pub fn name_kind(name: &str) -> Option<Kind> {
    let has = |list: &[&str]| list.iter().any(|n| n.eq_ignore_ascii_case(name.trim()));
    if has(&WOMEN) {
        Some(Kind::Woman)
    } else if has(&MEN) {
        Some(Kind::Man)
    } else {
        None
    }
}

pub fn voice_kind(voice: &str) -> Option<Kind> {
    let has = |list: &[&str]| list.iter().any(|n| n.eq_ignore_ascii_case(voice.trim()));
    if has(&WOMEN_VOICES) {
        Some(Kind::Woman)
    } else if has(&MEN_VOICES) {
        Some(Kind::Man)
    } else {
        None
    }
}

pub fn voices_for<'a>(voices: &[&'a str], kind: Kind) -> Vec<&'a str> {
    let fit: Vec<&str> = voices.iter().copied().filter(|v| voice_kind(v) == Some(kind)).collect();
    if fit.is_empty() { voices.to_vec() } else { fit }
}

pub fn seed_of(text: &str, salt: u64) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325 ^ salt, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

pub fn choose<'a>(list: &[&'a str], taken: &HashSet<String>, seed: u64) -> Option<&'a str> {
    if list.is_empty() {
        return None;
    }
    let start = (seed % list.len() as u64) as usize;
    let free = (0..list.len()).map(|i| list[(start + i) % list.len()]).find(|v| !taken.contains(&v.to_lowercase()));
    Some(free.unwrap_or(list[start]))
}

pub fn persona_for(id: &str, salt: u64, voices: &[&str], taken_names: &HashSet<String>, taken_voices: &HashSet<String>) -> Option<(String, String)> {
    let kind = if seed_of(id, salt ^ 0x51).is_multiple_of(2) { Kind::Woman } else { Kind::Man };
    let names: &[&str] = match kind {
        Kind::Woman => &WOMEN,
        Kind::Man => &MEN,
    };
    let name = choose(names, taken_names, seed_of(id, salt))?;
    let voice = voice_of(id, salt, voices, kind, taken_voices)?;
    Some((name.to_string(), voice))
}

pub fn voice_of(id: &str, salt: u64, voices: &[&str], kind: Kind, taken: &HashSet<String>) -> Option<String> {
    let fit = voices_for(voices, kind);
    choose(&fit, taken, seed_of(id, salt.rotate_left(17) ^ 0x9e37_79b9)).map(str::to_string)
}

pub fn wrong_voice(name: &str, voice: Option<&str>) -> Option<Kind> {
    let kind = name_kind(name)?;
    match voice.and_then(voice_kind) {
        Some(v) if v == kind => None,
        _ => Some(kind),
    }
}

pub fn voices_of(app: &App) -> Vec<String> {
    let deps = crate::voice_map::deps(app);
    let nixa = deps.nixa_voice.to_lowercase();
    let list = deps.known.filter(|l| !l.is_empty()).unwrap_or(deps.fallback);
    list.into_iter().filter(|v| v.to_lowercase() != nixa).collect()
}

pub fn voices(app: &App) -> HashMap<String, String> {
    lock(&app.watch)
        .world
        .iter()
        .filter_map(|(id, seen)| seen.info.labels.get(VOICE_LABEL).map(|v| (id.clone(), v.clone())))
        .collect()
}

pub async fn fill(app: &App) {
    let (missing, wrong, mut names, mut used) = {
        let watch = lock(&app.watch);
        let mut names = HashSet::new();
        let mut used = HashSet::new();
        let mut missing = Vec::new();
        let mut wrong = Vec::new();
        for (id, seen) in &watch.world {
            let voice = seen.info.labels.get(VOICE_LABEL);
            match seen.info.labels.get(NAME_LABEL) {
                Some(name) => {
                    names.insert(name.to_lowercase());
                    if let Some(kind) = wrong_voice(name, voice.map(String::as_str)) {
                        wrong.push((id.clone(), name.clone(), kind));
                    }
                }
                None => missing.push(id.clone()),
            }
            if let Some(voice) = voice {
                used.insert(voice.to_lowercase());
            }
        }
        (missing, wrong, names, used)
    };
    if missing.is_empty() && wrong.is_empty() {
        return;
    }
    let list = voices_of(app);
    let list: Vec<&str> = list.iter().map(String::as_str).collect();
    let salt = now_ms();
    for (id, name, kind) in wrong {
        let Some(voice) = voice_of(&id, salt, &list, kind, &used) else { continue };
        let Ok(term) = id.parse::<TermId>() else { continue };
        if engine::label(&term, VOICE_LABEL, Some(&voice)).await.is_err() {
            continue;
        }
        if let Some(seen) = lock(&app.watch).world.get_mut(&id) {
            seen.info.labels.insert(VOICE_LABEL.into(), voice.clone());
        }
        log("persona", &format!("{id} ({name}) now has voice {voice}"));
        used.insert(voice.to_lowercase());
    }
    for id in missing {
        let Some((name, voice)) = persona_for(&id, salt, &list, &names, &used) else { continue };
        let Ok(term) = id.parse::<TermId>() else { continue };
        if engine::label(&term, NAME_LABEL, Some(&name)).await.is_err() {
            continue;
        }
        let _ = engine::label(&term, VOICE_LABEL, Some(&voice)).await;
        if let Some(seen) = lock(&app.watch).world.get_mut(&id) {
            seen.info.labels.insert(NAME_LABEL.into(), name.clone());
            seen.info.labels.insert(VOICE_LABEL.into(), voice.clone());
        }
        log("persona", &format!("{id} is {name}, voice {voice}"));
        names.insert(name.to_lowercase());
        used.insert(voice.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_skips_taken_names_and_voices() {
        let voices = ["Harper", "Miles"];
        let names: HashSet<String> = WOMEN.iter().chain(MEN.iter()).skip(1).map(|n| n.to_lowercase()).collect();
        let used: HashSet<String> = HashSet::new();
        let (name, voice) = persona_for("m3", 7, &voices, &names, &used).unwrap();
        assert_eq!(name_kind(&name), voice_kind(&voice));
        assert!(persona_for("m3", 7, &[], &names, &used).is_none());
    }

    #[test]
    fn names_and_voices_match() {
        for id in ["m1", "m2", "m3", "m4", "m5", "m6"] {
            let (name, voice) = persona_for(id, 9, &["Harper", "Miles", "Freya", "Hugh"], &HashSet::new(), &HashSet::new()).unwrap();
            assert_eq!(name_kind(&name), voice_kind(&voice), "{name} {voice}");
        }
        assert_eq!(wrong_voice("Bodhi", Some("Zoey")), Some(Kind::Man));
        assert_eq!(wrong_voice("Bodhi", Some("Hugh")), None);
        assert_eq!(wrong_voice("Maya", None), Some(Kind::Woman));
        assert_eq!(wrong_voice("Zed", Some("Zoey")), None);
    }
}
