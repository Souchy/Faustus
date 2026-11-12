use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub group: String,
    pub sources: Vec<String>,
}

pub fn categories() -> Vec<Category> {
    let groups: &[(&str, &[(&str, &str, bool)])] = &[
        (
            "General",
            &[
                ("Currency", "Currency", true),
                ("Fragment", "Fragments", true),
                ("Wombgift", "Wombgifts", false),
                ("Runegraft", "Runegrafts", true),
                ("AllflameEmber", "Allflame Embers", true),
                ("Corpse", "Corpses", false),
                ("Tattoo", "Tattoos", true),
                ("Omen", "Omens", true),
                ("DjinnCoin", "Djinn Coins", true),
                ("Ducat", "Ducats", true),
                ("EnshroudingCrystal", "Enshrouding Crystals", true),
                ("DivinationCard", "Divination Cards", true),
                ("Artifact", "Artifacts", true),
                ("Oil", "Oils", true),
                ("Incubator", "Incubators", false),
            ],
        ),
        (
            "Equipment & Gems",
            &[
                ("UniqueWeapon", "Unique Weapons", false),
                ("UniqueArmour", "Unique Armours", false),
                ("UniqueAccessory", "Unique Accessories", false),
                ("UniqueFlask", "Unique Flasks", false),
                ("UniqueJewel", "Unique Jewels", false),
                ("ForbiddenJewel", "Forbidden Jewels", false),
                ("ShrineBelt", "Shrine Belts", false),
                ("UniqueTincture", "Unique Tinctures", false),
                ("UniqueRelic", "Unique Relics", false),
                ("SkillGem", "Skill Gems", false),
                ("ImbuedGem", "Imbued Gems", false),
                ("ClusterJewel", "Cluster Jewels", false),
            ],
        ),
        (
            "Atlas",
            &[
                ("Map", "Maps", false),
                ("BlightedMap", "Blighted Maps", false),
                ("BlightRavagedMap", "Blight-ravaged Maps", false),
                ("UniqueMap", "Unique Maps", false),
                ("ValdoMap", "Valdo Maps", false),
                ("DeliriumOrb", "Delirium Orbs", true),
                ("Invitation", "Invitations", false),
                ("Scarab", "Scarabs", true),
                ("Astrolabe", "Astrolabes", true),
                ("Memory", "Memories", false),
                ("IncursionTemple", "Temples", false),
                ("ScryingOrb", "Scrying Orbs", false),
            ],
        ),
        (
            "Crafting",
            &[
                ("BaseType", "Base Types", false),
                ("Flask", "Flasks", false),
                ("Fossil", "Fossils", true),
                ("Resonator", "Resonators", true),
                ("Beast", "Beasts", false),
                ("Essence", "Essences", true),
                ("Vial", "Vials", false),
            ],
        ),
    ];
    groups
        .iter()
        .flat_map(|(group, entries)| {
            entries.iter().map(|(id, name, exchange)| {
                let sources = if *exchange {
                    if matches!(*id, "Currency" | "Fragment") {
                        vec!["exchange", "stash"]
                    } else {
                        vec!["exchange"]
                    }
                } else {
                    vec!["stash"]
                };
                Category {
                    id: (*id).into(),
                    name: (*name).into(),
                    group: (*group).into(),
                    sources: sources.into_iter().map(str::to_owned).collect(),
                }
            })
        })
        .collect()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Line {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub details_id: Option<String>,
    pub price: Option<String>,
    pub divine_price: Option<String>,
    pub change: Option<String>,
    pub sparkline: Vec<Option<String>>,
    pub volume: Option<String>,
    pub pair: Option<String>,
    pub pair_rate: Option<String>,
    pub listings: Option<u64>,
    pub samples: Option<u64>,
    pub variant: Option<String>,
    pub base_type: Option<String>,
    pub item_type: Option<String>,
    pub links: Option<u64>,
    pub gem_level: Option<u64>,
    pub gem_quality: Option<u64>,
    pub corrupted: Option<bool>,
    pub map_tier: Option<u64>,
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Overview {
    pub league: String,
    pub category: String,
    pub source: String,
    pub fetched_at: String,
    pub quote_currency: String,
    pub rates: BTreeMap<String, String>,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct League {
    pub id: String,
    pub name: String,
    pub live: bool,
    pub archived: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Point {
    pub date: String,
    pub price: String,
    pub currency: String,
    pub granularity: String,
    #[serde(default)]
    pub volume: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Coverage {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct History {
    pub points: Vec<Point>,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Archive {
    pub name: String,
    pub status: String,
    pub rows: u64,
    pub skipped: u64,
    pub error: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub league: Option<String>,
    pub progress: String,
    pub license: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub category: String,
    pub line: Line,
    pub point: Point,
}
