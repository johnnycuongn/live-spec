//! The catalogue of note sections. A tab picks a subset; the prompt, the lane
//! split, both Markdown parsers and the fact-check key on it. Mirrors
//! `SECTIONS` in `src/lib/sections.ts`: ids, order, the default set and the
//! heading alias table must match.

#[derive(Debug, PartialEq, Eq)]
pub struct SectionSpec {
    /// Stable id: stored in tab files and templates, sent over IPC.
    pub id: &'static str,
    /// The heading the model writes (`## Conflicts`).
    pub heading: &'static str,
    /// What the section asks for, in the system prompt.
    pub guide: &'static str,
    /// Fact-checked against the code when present.
    pub checkable: bool,
}

/// Every section, in the order the notes show them. A tab picks a subset.
pub const CATALOGUE: [SectionSpec; 5] = [
    SectionSpec {
        id: "intent",
        heading: "Intent",
        guide: "What the spec asks for, for whom, and what done looks like, in one or two bullets. \
If the spec doesn't say, name what it fails to say.",
        checkable: false,
    },
    SectionSpec {
        id: "conflicts",
        heading: "Conflicts",
        guide: "Where the spec contradicts how the project works, re-does something that exists, or assumes wrongly. \
Cite file paths when the project context supports it. If nothing is worth raising, write \"- Nothing so far.\"",
        checkable: true,
    },
    SectionSpec {
        id: "gaps",
        heading: "Gaps",
        guide: "What an agent implementing this spec would need but can't find in it: acceptance criteria it could turn \
into tests, non-goals, error and edge cases, data or state the project doesn't have, commands to run. \
Name the gap, not a fix.",
        checkable: false,
    },
    SectionSpec {
        id: "ambiguities",
        heading: "Ambiguities",
        guide: "Phrases an agent would have to guess at. Quote the phrase, give the two readings, and propose precise wording.",
        checkable: false,
    },
    SectionSpec {
        id: "questions",
        heading: "Questions",
        guide: "One to three questions whose answers would most change the spec.",
        checkable: false,
    },
];

/// What a tab with no stored choice gets: the whole catalogue.
pub const DEFAULT_SECTIONS: [&str; 5] = ["intent", "conflicts", "gaps", "ambiguities", "questions"];

pub fn find(id: &str) -> Option<&'static SectionSpec> {
    CATALOGUE.iter().find(|s| s.id == id)
}

/// Known ids in catalogue order, each once. Empty stays empty: the default set.
pub fn normalize(ids: &[String]) -> Vec<String> {
    CATALOGUE.iter().filter(|s| ids.iter().any(|id| id == s.id)).map(|s| s.id.to_owned()).collect()
}

/// The specs a tab asked for, normalized. Nothing known (an empty list, or
/// ids a later build dropped) means the default set, never zero sections.
pub fn resolve(ids: &[String]) -> Vec<&'static SectionSpec> {
    let known: Vec<&'static SectionSpec> = CATALOGUE.iter().filter(|s| ids.iter().any(|id| id == s.id)).collect();
    if known.is_empty() {
        CATALOGUE.iter().collect()
    } else {
        known
    }
}

/// Which section a heading's first word opens, as an exact word (singular or
/// plural). "Faults" is the old build's name for Conflicts, so saved notes
/// still parse. Mirrors `keyFor` in `src/lib/sections.ts`.
pub fn heading_id(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "intent" => Some("intent"),
        "conflict" | "conflicts" | "fault" | "faults" => Some("conflicts"),
        "gap" | "gaps" => Some("gaps"),
        "ambiguity" | "ambiguities" => Some("ambiguities"),
        "question" | "questions" => Some("questions"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<'a>(specs: &[&'a SectionSpec]) -> Vec<&'a str> {
        specs.iter().map(|s| s.id).collect()
    }

    #[test]
    fn the_catalogue_is_the_default_set_in_order() {
        assert_eq!(ids(&CATALOGUE.iter().collect::<Vec<_>>()), DEFAULT_SECTIONS);
        assert_eq!(DEFAULT_SECTIONS, ["intent", "conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(CATALOGUE.iter().filter(|s| s.checkable).map(|s| s.id).collect::<Vec<_>>(), ["conflicts"]);
    }

    #[test]
    fn resolve_keeps_known_ids_in_catalogue_order_once_each() {
        let picked = resolve(&["questions".into(), "intent".into(), "questions".into(), "bogus".into()]);
        assert_eq!(ids(&picked), ["intent", "questions"]);
        assert_eq!(normalize(&["questions".into(), "intent".into(), "questions".into()]), ["intent", "questions"]);
    }

    #[test]
    fn nothing_known_means_the_default_set() {
        assert_eq!(ids(&resolve(&[])), DEFAULT_SECTIONS);
        assert_eq!(ids(&resolve(&["bogus".into()])), DEFAULT_SECTIONS, "a hand-edited file falls back, never to zero sections");
        assert!(normalize(&[]).is_empty(), "stored as empty: the default set");
        assert!(normalize(&["bogus".into()]).is_empty());
    }

    #[test]
    fn heading_words_map_exactly_including_the_old_builds_names() {
        for (word, id) in [
            ("Intent", "intent"),
            ("conflicts", "conflicts"),
            ("Conflict", "conflicts"),
            ("Faults", "conflicts"),
            ("fault", "conflicts"),
            ("Gaps", "gaps"),
            ("gap", "gaps"),
            ("Ambiguities", "ambiguities"),
            ("ambiguity", "ambiguities"),
            ("Questions", "questions"),
            ("question", "questions"),
        ] {
            assert_eq!(heading_id(word), Some(id), "{word}");
        }
        for word in ["Intentional", "Gapless", "Directions", "Summary", ""] {
            assert_eq!(heading_id(word), None, "{word}");
        }
        assert_eq!(find("gaps").map(|s| s.heading), Some("Gaps"));
        assert!(find("directions").is_none(), "the old build's section is not in the catalogue");
    }
}
