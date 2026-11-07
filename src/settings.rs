use std::collections::HashMap;
use std::path::PathBuf;

/// User settings persisted as a JSON key/value store (mirrors opti's Electron
/// data.json, which is a generic `data[key]` map in the app's userData dir).
#[derive(Clone, Default)]
pub struct Settings {
    map: HashMap<String, serde_json::Value>,
}

impl Settings {
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.map.get(key).and_then(|v| v.as_f64())
    }

    pub fn set_f64(&mut self, key: &str, value: f64) {
        self.map.insert(key.to_string(), serde_json::json!(value));
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.map.get(key).and_then(|v| v.as_i64()).map(|i| i as i32)
    }

    pub fn set_i32(&mut self, key: &str, value: i32) {
        self.map.insert(key.to_string(), serde_json::json!(value));
    }

    #[allow(dead_code)]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.map.get(key).and_then(|v| v.as_bool())
    }

    #[allow(dead_code)]
    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.map.insert(key.to_string(), serde_json::json!(value));
    }

    pub fn volume(&self) -> f64 {
        self.get_f64("volume").unwrap_or(0.5)
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.set_f64("volume", volume.clamp(0.0, 1.0));
    }

    pub fn load() -> Self {
        std::fs::read(data_file())
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, serde_json::Value>>(&b).ok())
            .map(|map| Self { map })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = data_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.map) {
            let _ = std::fs::write(path, json);
        }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_default()
        })
}

pub fn data_file() -> PathBuf {
    config_dir().join("opti").join("data.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip() {
        let mut s = Settings::default();
        s.set_f64("volume", 0.25);
        s.set_i32("bounds_w", 1234);
        s.set_bool("pinned", true);
        let json = serde_json::to_string_pretty(&s.map).unwrap();
        let loaded: HashMap<String, serde_json::Value> = serde_json::from_str(&json).unwrap();
        let restored = Settings { map: loaded };
        assert_eq!(restored.get_f64("volume"), Some(0.25));
        assert_eq!(restored.get_i32("bounds_w"), Some(1234));
        assert_eq!(restored.get_bool("pinned"), Some(true));
    }

    #[test]
    fn volume_clamps() {
        let mut s = Settings::default();
        s.set_volume(5.0);
        assert_eq!(s.volume(), 1.0);
        s.set_volume(-2.0);
        assert_eq!(s.volume(), 0.0);
    }

    #[test]
    fn missing_keys_default() {
        let s = Settings::default();
        assert_eq!(s.volume(), 0.5);
        assert_eq!(s.get_i32("nope"), None);
        assert_eq!(s.get_bool("nope"), None);
    }
}