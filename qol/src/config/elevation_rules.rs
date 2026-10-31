use serde::{Deserialize, Serialize};

/// Firing-arc rules for Steam 1.163, independent of manual editor elevation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElevationRules {
    /// Keep the running game's rules unchanged (default).
    #[default]
    Vanilla,
    /// Restore the 1.151 same-floor gun blocking and KAZ cone/coverage rules.
    Classic,
    /// Classic rules, additionally ignoring MDL_FERMA trusses for KAZ arc blocking.
    #[serde(rename = "classic_kaz")]
    ClassicKaz,
    /// Keep the 1.163 rules unchanged, like vanilla on the supported game version.
    Modern,
}

impl ElevationRules {
    /// Whether to prepare the classic gun and KAZ patches.
    pub fn is_classic(self) -> bool {
        matches!(self, Self::Classic | Self::ClassicKaz)
    }

    /// Whether KAZ ray scans should ignore trusses.
    pub fn exempts_trusses(self) -> bool {
        self == Self::ClassicKaz
    }
}
