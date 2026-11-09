//! The mod's `ally.json` settings, their defaults and the legacy-key migrations.
//!
//! Loading is owned by [`load`]: it reads the file, writes the defaults on first run and logs a
//! parse error before falling back to the defaults.

use serde::{Deserialize, Serialize};

/// Where the modloader expects the mod's settings.
pub(crate) const PATH: &str = "Modloader/config/ally.json";

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    /// Master switch. When true, every gameplay feature is installed, including the wingman.
    /// When false, no gameplay hooks are installed; the crash reporter still loads.
    pub(crate) enabled: bool,
    /// Governed dive, native-aimed whole-payload release and pull-out for bomb-carrying allies.
    pub(crate) ally_bombers: bool,
    /// Game seconds the player's ship must have been in the battle before the ally arrives.
    pub(crate) ally_delay_seconds: f64,
    /// What to do when the flagship leaves the arena alive: "handover" (default) = the ally is promoted to your
    /// ship and the retreat point the flagship used up is put back, so the new flagship can still retreat;
    /// "retreat" = the ally is extracted with it and the battle ends as a retreat. An invalid value logs a
    /// warning and uses "handover".
    pub(crate) flagship_leaves: String,
    /// Virtual-key code of the "ally, retreat" key (0x77 = F8); 0 disables the key.
    pub(crate) retreat_key: u32,
    /// How many AI allies may fight at the same time. Ships switched on beyond that wait in the reserve and
    /// step in, in ship-number order, when an ally is destroyed or has left.
    pub(crate) max_allies: u32,
    /// 1/2 take the displayed top/bottom ally; TAB cycles deployed friendly ships.
    pub(crate) ship_switching: bool,
    /// Allies never keep a prize ship (the ships the player may capture and sell after the battle: the game flags
    /// them at creature +0xc19) as a target: they pick another enemy, or hold fire when only prizes are left.
    pub(crate) spare_prizes: bool,
    /// Let AI-run allies launch their own ship missiles (the native Zenith launch path). False = an ally
    /// controlled by the AI holds its missile racks; the ship you fly can still launch manually.
    /// Ship bombers always suppress launches during a bombing course.
    pub(crate) ally_zenith_autofire: bool,
    /// Allies roll the AI's AP / proximity shell doctrine once, like enemy ships do at spawn. Default on.
    pub(crate) ally_ai_ammo: bool,
    /// Damage popups of allied ships are drawn in this colour, as HEX: `"#00E4FF"` (`"00E4FF"` and `"0x00E4FF"` work
    /// too). A plain number is still accepted so configs from before v0.14.0 keep working (read as 0xRRGGBB).
    #[serde(deserialize_with = "deserialize_hex_color")]
    pub(crate) ally_damage_color: String,
    /// Draw a half-size ship panel for each ally beside the player's own panel while the ally is in the battle.
    pub(crate) ally_panels: bool,
    /// Ally target box size relative to the native 62 x 62 frame. 0 = off; positive values
    /// scale the box; negative or non-finite values fall back to 0.6.
    pub(crate) ally_reticle_scale: f32,
    /// Radio sounds for ally events: the retreat key, an ally joining the fight, an ally destroyed. Lines follow
    /// the game's language: Russian from `Media/Snd/RADIO`, English from `Media/Snd/RADIO_ENG`.
    pub(crate) ally_sounds: bool,
    /// An ally leaves the battle (same order as the retreat key) when its COMBAT fuel (the ship's own tank, the
    /// one the battle burns; not the map fuel of the fleet) falls to this percent. 0 = off. Allies only.
    pub(crate) ally_fuel_retreat_percent: f32,
    /// Share of the player's battle honor/XP given to every deployed ally: 0 = off, 0.5 = 50% (default),
    /// 1 = 100%. The player keeps the full award; each ally receives its own `share * player award`.
    pub(crate) ally_honor_share: f32,
    /// Pre-combat button that picks the armament of a carrier's planes (charged to the inventory). Default on.
    pub(crate) plane_loadouts: bool,
    /// Give a chosen bomb / rocket loadout a 37 mm gunpod and keep the plane in the fight after the payload is gone:
    /// once bombs and rockets are empty it switches to the interceptor mode and strafes with the gun until that is dry.
    /// Normal tactical battle only (`Combat.seria`; `stateSelectorAircraft_ == 4`), never the strategic bomber scene.
    /// Default on.
    pub(crate) extra_gunpod: bool,
    /// Offer the K-13 air-to-air loadout for carrier planes. Enabled in the player default preset.
    pub(crate) plane_k13: bool,
    pub(crate) plane_attack_delay: f32,
    pub(crate) plane_aa_range: f32,
    pub(crate) plane_aa_delay: f32,
    pub(crate) plane_aa_angle: f32,
    pub(crate) plane_aa_speed: f32,
    /// Enemy slots active at once (vanilla 3, 1..=40). `Battle_DeployFromReserve` uses this for the
    /// initial group and for the refill after the 30 s timer. The weight budget is `3 * group_size`.
    pub(crate) enemy_group_size: u32,
    /// Text drawn next to ship number 1 on the pre-combat screen (the flagship, always your own ship); "" = none.
    pub(crate) marker_text: String,
    /// Diagnostic switches grouped for the config file.
    pub(crate) logging: LoggingConfig,
    /// Battle debug overlay; its own settings block, see `docs/debug-overlay.md`.
    pub(crate) debug_overlay: DebugOverlayConfig,
    /// SIDE PATCH (not in master). `m_oid`s the AI never treats as a strategic launch to intercept in the missile-vs-ship
    /// arcade scene, e.g. `["MDL_MISSILE_03"]` (the R-9 Sprint) for a Sprint modded to be strat-launchable (its category has bit 0x8000000, so
    /// the CIWS and anti-air launchers of both sides would shoot at it). Copy the names from `m_oid=` in OL.seria. Empty = vanilla.
    pub(crate) intercept_ignore_oids: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            ally_bombers: true,
            ally_delay_seconds: 1.5,
            flagship_leaves: "handover".to_string(),
            retreat_key: 0x77,
            max_allies: 2,
            ship_switching: true,
            spare_prizes: true,
            ally_zenith_autofire: true,
            ally_ai_ammo: true,
            ally_damage_color: "#00E4FF".to_string(),
            ally_panels: true,
            ally_reticle_scale: 0.6,
            ally_sounds: true,
            ally_fuel_retreat_percent: 30.0,
            ally_honor_share: 0.5,
            plane_loadouts: true,
            extra_gunpod: true,
            plane_k13: true,
            plane_attack_delay: 1.0,
            plane_aa_range: 6000.0,
            plane_aa_delay: 1.0,
            plane_aa_angle: 0.35,
            plane_aa_speed: 1.5,
            enemy_group_size: 3,
            marker_text: "PLAYER".to_string(),
            logging: LoggingConfig::default(),
            debug_overlay: DebugOverlayConfig::default(),
            intercept_ignore_oids: vec!["MDL_MISSILE_03".to_string()],
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LoggingConfig {
    /// Runtime debug mode: megalog, watchdog, crash minidump and the verbose diagnostics.
    /// Off by default and independent of the cargo feature.
    pub(crate) debug_log: bool,
    /// Full GUI lists on clicks and native ammo-selector rebuilds (diagnostic, off by default).
    pub(crate) gui_inspect: bool,
    /// Creature add/remove log (diagnostic, off by default).
    pub(crate) log_creatures: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            debug_log: false,
            gui_inspect: false,
            log_creatures: false,
        }
    }
}

/// Battle debug overlay settings; the `debug_overlay` block in `ally.json` (own category since
/// v1.0.1; before that it was the `logging.debug_overlay` boolean).
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DebugOverlayConfig {
    /// Master switch. Off by default; independent of the ally reticles.
    pub(crate) enable: bool,
    /// Ring radii in world units around every live ship; they shrink and grow with the battle
    /// camera. An empty list draws no rings; invalid radii are skipped.
    pub(crate) circles: Vec<f32>,
    /// Draw each AI ship's steering waypoint: the marker, its dotted lead-in and the distance.
    pub(crate) ai_waypoint: bool,
    /// Estimated barrel bearing for the first live gun's supported unguided cannon ammo.
    pub(crate) prediction_marker: bool,
    /// Ordinary native aim-ray endpoint and overlay bearing-marker distance, in world units.
    pub(crate) pointer_length: f32,
}

impl Default for DebugOverlayConfig {
    fn default() -> Self {
        Self {
            enable: false,
            circles: vec![250.0, 500.0, 1000.0],
            ai_waypoint: true,
            prediction_marker: false,
            pointer_length: 300.0,
        }
    }
}

/// Read the config, falling back to the defaults (and writing them on first run).
pub(crate) fn load() -> Config {
    let path = PATH;
    match std::fs::read_to_string(path) {
        Ok(text) => parse_config(&text).unwrap_or_else(|error| {
            log::error!("hf_ally: bad {path}: {error}; using defaults");
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            if let Ok(text) = serde_json::to_string_pretty(&config) {
                let _ = std::fs::write(path, text);
            }
            config
        }
    }
}

/// Accept old config keys; explicit nested settings take precedence.
fn parse_config(text: &str) -> Result<Config, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(root) = value.as_object_mut() {
        migrate_legacy_logging(root);
        migrate_legacy_debug_overlay(root);
    }
    serde_json::from_value(value)
}

/// Old flat logging keys move into the `logging` block; a key already there wins.
fn migrate_legacy_logging(root: &mut serde_json::Map<String, serde_json::Value>) {
    let mut legacy = serde_json::Map::new();
    for key in ["debug_log", "gui_inspect", "log_creatures"] {
        if let Some(setting) = root.remove(key) {
            legacy.insert(key.to_string(), setting);
        }
    }
    let logging = root
        .entry("logging")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(logging) = logging.as_object_mut() {
        for (key, setting) in legacy {
            logging.entry(key).or_insert(setting);
        }
    }
}

/// `debug_overlay` moved out of `logging` into its own block (v1.0.1). A legacy boolean — the old
/// flat `debug_overlay` key or `logging.debug_overlay` — becomes the new block's `enable`; an
/// explicit new block wins over both.
fn migrate_legacy_debug_overlay(root: &mut serde_json::Map<String, serde_json::Value>) {
    let flat = match root.get_mut("debug_overlay") {
        Some(serde_json::Value::Bool(_)) => root.remove("debug_overlay"),
        Some(_) => return, // the explicit new block wins; drop only the stale grouped boolean
        None => None,
    };
    let grouped = root
        .get_mut("logging")
        .and_then(|logging| logging.as_object_mut())
        .and_then(|logging| logging.remove("debug_overlay"));
    if root.contains_key("debug_overlay") {
        return;
    }
    let Some(setting) = flat.or(grouped) else {
        return;
    };
    root.insert(
        "debug_overlay".to_string(),
        serde_json::json!({ "enable": setting }),
    );
}

/// Read `ally_damage_color` from the config: a HEX string (`"#00E4FF"`, `"00E4FF"` or `"0x00E4FF"`), or a plain
/// number from a pre-v0.14.0 config, kept as HEX so the file the mod writes shows the colour the player knows.
fn deserialize_hex_color<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HexColor {
        Text(String),
        Number(u64),
    }
    Ok(match HexColor::deserialize(deserializer)? {
        HexColor::Text(text) => text,
        HexColor::Number(value) => format!("#{:06X}", value & 0x00ff_ffff),
    })
}

/// Parse a HEX colour (`#RRGGBB`, `RRGGBB` or `0xRRGGBB`; alphabet is case-insensitive; 8 digits are AARRGGBB and the
/// alpha is dropped) into the game's 0xRRGGBB value.
pub(crate) fn parse_hex_color(text: &str) -> Option<u32> {
    let hex = text.trim();
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let hex = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .map(|value| value & 0x00ff_ffff)
}
