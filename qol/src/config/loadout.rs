use highfleet::general::EscadraString;
use serde::{de::Deserializer, Deserialize, Serialize};

use crate::structs::{
    cvec::CVec,
    loadout::{GameItemMunition, GameLoadout},
};

const LEGACY_GUN_AMMO: &str = "ITEM_GUN37";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
/// One type of bomb, rocket, missile, or other carried item in an aircraft loadout.
pub struct ConfigItemMunition {
    /// Internal item name used by the game, such as `ITEM_K13`.
    pub name: EscadraString,
    /// Number of this item carried by each aircraft.
    pub count: u32,
}

#[derive(Debug, Default, Clone, Serialize)]
/// A selectable collection of weapons and ammunition for an aircraft.
pub struct ConfigLoadout {
    /// Unique internal name for this loadout.
    pub oid: EscadraString,
    /// Resource name of the icon shown for this loadout.
    pub icon: EscadraString,
    /// Bombs, rockets, missiles, and other counted items carried by each aircraft.
    pub vec_parts: Vec<ConfigItemMunition>,
    /// How likely it is for the AI to use this loadout.
    ///
    /// The percentage gets calculated as `weight of this loadout / total weight of all loadouts`.
    pub launch_loadout_weight: u32,
    /// Internal name of the ammunition used by the aircraft's built-in gun.
    ///
    /// For example, use `ITEM_GUN37` for the 37 mm gun or `ITEM_GUN57` for the 57 mm gun.
    /// Leave this unset for a loadout without a gun.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gun_ammo: Option<EscadraString>,
}

impl<'de> Deserialize<'de> for ConfigLoadout {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            oid: EscadraString,
            icon: EscadraString,
            vec_parts: Vec<ConfigItemMunition>,
            launch_loadout_weight: u32,
            #[serde(default)]
            gun_ammo: Option<EscadraString>,
            #[serde(default)]
            has_gun37mm: bool,
        }

        let serialized = Wire::deserialize(deserializer)?;
        let gun_ammo = serialized.gun_ammo.or_else(|| {
            serialized
                .has_gun37mm
                .then(|| EscadraString::from(LEGACY_GUN_AMMO))
        });

        Ok(Self {
            oid: serialized.oid,
            icon: serialized.icon,
            vec_parts: serialized.vec_parts,
            launch_loadout_weight: serialized.launch_loadout_weight,
            gun_ammo,
        })
    }
}

impl From<&GameItemMunition> for ConfigItemMunition {
    fn from(item: &GameItemMunition) -> Self {
        Self {
            name: item.name.clone(),
            count: item.count,
        }
    }
}

impl From<&ConfigItemMunition> for GameItemMunition {
    fn from(item: &ConfigItemMunition) -> Self {
        Self {
            name: item.name.clone(),
            count: item.count,
            _padding: [0; 4],
        }
    }
}

impl From<&GameLoadout> for ConfigLoadout {
    fn from(loadout: &GameLoadout) -> Self {
        Self {
            oid: loadout.oid.clone(),
            icon: loadout.icon.clone(),
            vec_parts: loadout
                .vec_parts
                .items()
                .into_iter()
                .map(ConfigItemMunition::from)
                .collect(),
            launch_loadout_weight: loadout.launch_loadout_weight,
            gun_ammo: loadout
                .has_gun37mm
                .then(|| EscadraString::from(LEGACY_GUN_AMMO)),
        }
    }
}

impl From<&ConfigLoadout> for GameLoadout {
    fn from(loadout: &ConfigLoadout) -> Self {
        let mut vec_parts = CVec::empty();
        for item in &loadout.vec_parts {
            vec_parts.insert(GameItemMunition::from(item));
        }

        Self {
            oid: loadout.oid.clone(),
            icon: loadout.icon.clone(),
            vec_parts,
            launch_loadout_weight: loadout.launch_loadout_weight,
            has_gun37mm: loadout.gun_ammo.is_some(),
            _padding: [0; 3],
        }
    }
}
