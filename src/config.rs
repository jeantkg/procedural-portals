use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Medium,
    High,
}

impl Quality {
    pub fn size(self) -> [u32; 3] {
        match self {
            Self::Low => [32, 24, 40],
            Self::Medium => [48, 32, 48],
            Self::High => [64, 40, 64],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub cached_terrain: bool,
    pub search_distance: f32,
    pub quality: Quality,
    pub seed: u32,
    pub ambient_light: u8,
    pub max_portals: usize,
    pub show_status: bool,
}

impl Default for Config {
    fn default() -> Self {
        serde_json::from_str(include_str!("../default.settings.json"))
            .expect("valid checked-in default settings")
    }
}

impl Config {
    /// Missing keys inherit the single checked-in defaults; invalid configuration fails closed.
    pub fn parse(json: &str) -> Result<Self, String> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        let mut defaults = serde_json::to_value(Self::default()).unwrap();
        let mut supplied: serde_json::Value =
            serde_json::from_str(json).map_err(|e| e.to_string())?;
        let object = supplied
            .as_object_mut()
            .ok_or("settings must be an object")?;
        // Previous prototypes used these keys. Upgrade their mode selection explicitly.
        if let Some(value) = object.remove("live_terrain") {
            if !value.is_boolean() {
                return Err("live_terrain must be boolean".into());
            }
            object.entry("cached_terrain").or_insert(value);
        }
        if let Some(value) = object.remove("load_destination_chunks")
            && !value.is_boolean()
        {
            return Err("load_destination_chunks must be boolean".into());
        }
        // Upgrade old companions without pretending that native terrain supports post-pass opacity.
        for (key, min, max) in [
            ("opacity", 0.0, 1.0),
            ("glow", 0.0, 2.0),
            ("animation_speed", 0.0, 3.0),
            ("fade_seconds", 0.1, 3.0),
        ] {
            if let Some(value) = object.remove(key) {
                let number = value.as_f64().ok_or("legacy setting must be numeric")?;
                if !(min..=max).contains(&number) {
                    return Err("invalid legacy setting".into());
                }
                if key == "opacity" && number == 0.0 {
                    object.insert("enabled".into(), false.into());
                }
                if key == "glow" && !object.contains_key("ambient_light") {
                    object.insert(
                        "ambient_light".into(),
                        ((7.0 + number * 4.0).round() as u8).min(15).into(),
                    );
                }
            }
        }
        for (key, value) in object {
            defaults[key] = value.clone();
        }
        let config: Self = serde_json::from_value(defaults).map_err(|e| e.to_string())?;
        for (value, min, max) in [(
            config.search_distance,
            8.0,
            mod_api::MAX_TERRAIN_DISTANCE as f32 - 16.0,
        )] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err("setting outside its supported range".into());
            }
        }
        if config.ambient_light > 15
            || !(1..=crate::portals::VISIBLE_PORTALS).contains(&config.max_portals)
        {
            return Err("invalid light level or portal count".into());
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_settings_use_defaults_and_invalid_values_are_refused() {
        let changed = Config::parse(r#"{"ambient_light":8}"#).unwrap();
        assert_eq!(changed.search_distance, Config::default().search_distance);
        assert_eq!(changed.ambient_light, 8);
        for invalid in [
            r#"{"enabled":1}"#,
            r#"{"unknown":true}"#,
            r#"{"opacity":2}"#,
            r#"{"search_distance":128}"#,
            r#"{"fade_seconds":0}"#,
            r#"{"max_portals":0}"#,
            r#"{"ambient_light":16}"#,
            r#"{"quality":"ultra"}"#,
            "null",
            "[]",
        ] {
            assert!(Config::parse(invalid).is_err(), "{invalid}");
        }
        Config::parse(include_str!("../default.settings.json")).unwrap();
        assert!(!Config::parse(r#"{"opacity":0}"#).unwrap().enabled);
    }

    #[test]
    fn previous_mode_keys_migrate_without_requesting_destination_loading() {
        assert!(
            Config::parse(r#"{"live_terrain":true,"load_destination_chunks":true}"#)
                .unwrap()
                .cached_terrain
        );
        assert!(
            !Config::parse(r#"{"live_terrain":false}"#)
                .unwrap()
                .cached_terrain
        );
        assert!(
            !Config::parse(r#"{"live_terrain":true,"cached_terrain":false}"#)
                .unwrap()
                .cached_terrain
        );
        assert!(Config::parse(r#"{"live_terrain":1}"#).is_err());
        assert!(Config::parse(r#"{"load_destination_chunks":"true"}"#).is_err());
    }
}
