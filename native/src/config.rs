use serde::{Deserialize, Serialize};

pub const PATH: &str = "Modloader/config/hf_native.json";

/// Each experimental feature is installed only at process startup.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub link_search: bool,
    pub tail_probe: bool,
    pub parts_direct_append: bool,
    pub contact_pool: bool,
    pub contact_pool_capacity: usize,
    pub diagnostics_interval_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            link_search: true,
            tail_probe: true,
            parts_direct_append: true,
            contact_pool: true,
            contact_pool_capacity: 4096,
            diagnostics_interval_seconds: 10,
        }
    }
}

impl Config {
    pub fn read() -> Result<Self, String> {
        let config = match std::fs::read_to_string(PATH) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|error| format!("cannot parse {PATH}: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                let text =
                    serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
                std::fs::write(PATH, text)
                    .map_err(|error| format!("cannot create {PATH}: {error}"))?;
                config
            }
            Err(error) => return Err(format!("cannot read {PATH}: {error}")),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if !(1..=65_536).contains(&self.contact_pool_capacity) {
            return Err("contact_pool_capacity must be in 1..=65536".into());
        }
        if self.diagnostics_interval_seconds != 0
            && !(1..=3600).contains(&self.diagnostics_interval_seconds)
        {
            return Err("diagnostics_interval_seconds must be 0 or in 1..=3600".into());
        }
        Ok(())
    }
}
