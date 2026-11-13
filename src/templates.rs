//! Шаблоны серверов (6 пресетов).
//! Идея и данные пресетов: Zomboid Control Panel (fpsacha/zomboid-control-panel, MIT).
//! Инварианты применения: валидация имени сервера, секреты/сеть не перезаписываются,
//! неизвестные ключи пропускаются, бэкап перед записью, откат при сбое второго файла.
use crate::config::ServerCfg;
use crate::{atomic, ini, sandbox, validate};
use serde_json::Value;
use std::collections::BTreeMap;

pub const EXCLUDED_INI_KEYS: &[&str] =
    &["RCONPassword", "Password", "ServerName", "PublicName", "DefaultPort", "UDPPort", "RCONPort", "server_browser_announced_ip"];

#[derive(Debug, Clone)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// категория -> ключ -> значение
    pub sandbox: BTreeMap<String, BTreeMap<String, String>>,
    pub ini: BTreeMap<String, String>,
}

fn parse(id: &str, src: &str) -> Template {
    let v: Value = serde_json::from_str(src).expect("встроенный шаблон должен разбираться");
    let meta = &v["meta"];
    let s = |x: &Value| x.as_str().unwrap_or("").to_string();
    let lua = |x: &Value| match x {
        Value::String(s) => s.clone(),
        o => o.to_string(),
    };
    let mut sandbox_vars = BTreeMap::new();
    if let Some(sv) = v["sandboxVars"].as_object() {
        for (cat, inner) in sv {
            let name = if cat == "settings" { sandbox::ROOT.to_string() } else { cat.clone() };
            let fields = inner.as_object().map(|o| o.iter().map(|(k, x)| (k.clone(), lua(x))).collect()).unwrap_or_default();
            sandbox_vars.insert(name, fields);
        }
    }
    let ini_map = v["serverIni"]
        .as_object()
        .map(|o| o.iter().filter(|(k, _)| !EXCLUDED_INI_KEYS.contains(&k.as_str())).map(|(k, x)| (k.clone(), lua(x))).collect())
        .unwrap_or_default();
    Template {
        id: id.to_string(),
        name: if meta["name"].is_string() { s(&meta["name"]) } else { id.to_string() },
        description: s(&meta["description"]),
        tags: meta["tags"].as_array().map(|a| a.iter().map(&s).collect()).unwrap_or_default(),
        sandbox: sandbox_vars,
        ini: ini_map,
    }
}

pub fn all() -> Vec<Template> {
    [
        ("vanilla-apocalypse", VANILLA),
        ("hardcore-survivor", HARDCORE),
        ("first-week-friendly", FIRST_WEEK),
        ("six-months-later", SIX_MONTHS),
        ("builders-paradise", BUILDERS),
        ("pvp-raiding", PVP_RAIDING),
    ]
    .into_iter()
    .map(|(id, src)| parse(id, src))
    .collect()
}

#[derive(Debug, Clone)]
pub struct Report {
    pub ini_applied: Vec<String>,
    pub sandbox_applied: usize,
    pub sandbox_total: usize,
}

/// Применяет шаблон. Сервер должен быть остановлен.
pub fn apply(cfg: &ServerCfg, running: bool, t: &Template) -> Result<Report, String> {
    if running {
        return Err("сервер запущен — останови его перед применением шаблона".into());
    }
    validate::server_name(&cfg.name)?;
    let ini_path = cfg.ini();
    let sb_path = sandbox::path(cfg);
    let kv: Vec<(String, String)> = t.ini.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let (ini_applied, ini_bak) = if ini_path.exists() && !kv.is_empty() {
        ini::set_existing(&ini_path, &kv).map_err(|e| format!("запись {}: {e}", ini_path.display()))?
    } else {
        (vec![], None)
    };
    let changes: Vec<(String, String, String)> =
        t.sandbox.iter().flat_map(|(c, f)| f.iter().map(move |(k, v)| (c.clone(), k.clone(), v.clone()))).collect();
    let mut sandbox_applied = 0;
    if sb_path.exists() && !changes.is_empty() {
        let text = std::fs::read_to_string(&sb_path).map_err(|e| e.to_string())?;
        let (new, n) = sandbox::update(&text, &changes);
        sandbox_applied = n;
        if let Err(e) = atomic::write(&sb_path, new.as_bytes()) {
            if let Some(b) = &ini_bak {
                let _ = std::fs::copy(b, &ini_path); // откат первого файла
            }
            return Err(format!("запись SandboxVars (ini откачен): {e}"));
        }
    }
    Ok(Report { ini_applied, sandbox_applied, sandbox_total: changes.len() })
}

const VANILLA: &str = r#"{"meta":{"id":"vanilla-apocalypse","name":"Vanilla Apocalypse","description":"Stock Project Zomboid defaults — the classic experience with no overrides. Use this as a clean baseline.","tags":["vanilla","balanced"]},"sandboxVars":{"settings":{"Zombies":4,"DayLength":16,"StartWeather":0}},"serverIni":{}}"#;

const HARDCORE: &str = r#"{"meta":{"id":"hardcore-survivor","name":"Hardcore Survivor","description":"A punishing, high-population run with scarce loot and no respawn boosts. Zombies are tougher, infection is nearly instant, and buildings are already picked over.","tags":["hardcore","permadeath-friendly"]},"sandboxVars":{"settings":{"Zombies":2,"ZombieRespawn":1,"FoodLootNew":0.4,"MedicalLootNew":0.3,"AmmoLootNew":0.3,"WeaponLootNew":0.3,"OtherLootNew":0.4,"MaximumLooted":60},"ZombieLore":{"Speed":2,"Strength":1,"Toughness":1,"Mortality":2,"Memory":1,"Sight":2,"Hearing":2},"ZombieConfig":{"PopulationMultiplier":1.2,"PopulationPeakMultiplier":2.0},"MultiplierConfig":{"Global":0.75}},"serverIni":{"Faction":false,"PlayerSafehouse":false}}"#;

const FIRST_WEEK: &str = r#"{"meta":{"id":"first-week-friendly","name":"First Week Friendly","description":"Gentler start: slower zombies, more loot, longer days. Good for learning the ropes.","tags":["friendly","beginner"]},"sandboxVars":{"settings":{"Zombies":5,"ZombieRespawn":2,"FoodLootNew":1.5,"MedicalLootNew":1.5,"WeaponLootNew":1.2,"DayLength":24},"ZombieLore":{"Speed":1,"Strength":1,"Toughness":1,"Mortality":1,"Memory":1,"Sight":1,"Hearing":1},"MultiplierConfig":{"Global":1.25}},"serverIni":{"Faction":true,"PlayerSafehouse":true}}"#;

const SIX_MONTHS: &str = r#"{"meta":{"id":"six-months-later","name":"Six Months Later","description":"Classic long-run preset: half a year into the apocalypse — water and power already gone, loot mostly taken.","tags":["long-run","classic"]},"sandboxVars":{"settings":{"Zombies":3,"ZombieRespawn":2,"WaterShut":0,"ElecShut":0,"MaximumLooted":100},"ZombieConfig":{"PopulationMultiplier":1.0,"PopulationPeakMultiplier":1.5}},"serverIni":{"Faction":true,"PlayerSafehouse":true}}"#;

const BUILDERS: &str = r#"{"meta":{"id":"builders-paradise","name":"Builders Paradise","description":"Build-focused sandbox: abundant resources, low zombie pressure, long days.","tags":["building","relaxed"]},"sandboxVars":{"settings":{"Zombies":6,"ZombieRespawn":4,"FoodLootNew":2.0,"MedicalLootNew":2.0,"AmmoLootNew":1.0,"WeaponLootNew":2.0,"DayLength":27},"ZombieLore":{"Speed":1,"Strength":1,"Toughness":1}},"serverIni":{"Faction":true,"PlayerSafehouse":true}}"#;

const PVP_RAIDING: &str = r#"{"meta":{"id":"pvp-raiding","name":"PvP Raiding","description":"Player-vs-player focus: safehouses on, factions on, loot economy tuned for raiding.","tags":["pvp","raiding"]},"sandboxVars":{"settings":{"Zombies":5,"ZombieRespawn":2,"FoodLootNew":1.0,"AmmoLootNew":0.7,"WeaponLootNew":0.8,"MaximumLooted":40}},"serverIni":{"Faction":true,"PlayerSafehouse":true}}"#;
