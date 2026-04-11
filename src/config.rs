// config.rs — loading and updating config.json, bookmarks.json, locales
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Returns the directory containing the running executable.
/// Falls back to current working directory if the path can't be resolved.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap())
}

/// Loads the locale JSON string for the language set in config.json.
/// Falls back to English if the configured locale file is missing.
pub fn load_locale() -> String {
    let dir = exe_dir();

    // Read language from config, default to "en"
    let config_path = dir.join("config.json");
    let config_str = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        fs::read_to_string("config.json").unwrap_or_else(|_| r#"{"language":"en"}"#.to_string())
    };

    let lang = serde_json::from_str::<Value>(&config_str)
        .ok()
        .and_then(|v| v.get("language").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| "en".to_string());

    // Try exe dir first, then cwd, then fall back to en.json
    let locale_path = dir.join(format!("locales/{}.json", lang));
    if locale_path.exists() {
        fs::read_to_string(&locale_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        fs::read_to_string(format!("locales/{}.json", lang))
            .unwrap_or_else(|_| fs::read_to_string("locales/en.json").unwrap_or_else(|_| "{}".to_string()))
    }
}

/// Loads config.json as a raw JSON string.
/// Returns "{}" if the file doesn't exist yet.
pub fn load_config() -> String {
    let dir = exe_dir();
    let config_path = dir.join("config.json");
    if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        fs::read_to_string("config.json").unwrap_or_else(|_| "{}".to_string())
    }
}

/// Updates a single key in a JSON config string and returns the updated string.
/// Creates the key if it doesn't exist; preserves all other keys.
pub fn update_config_value(config: &str, key: &str, value: &str) -> String {
    let mut root = serde_json::from_str::<Value>(config)
        .unwrap_or_else(|_| Value::Object(Default::default()));
    if let Value::Object(ref mut map) = root {
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".to_string())
}

/// Loads bookmarks.json as a raw JSON string.
/// Falls back to a hardcoded set of default bookmarks if the file is missing.
pub fn load_bookmarks() -> String {
    let dir = exe_dir();
    let bookmarks_path = dir.join("bookmarks.json");
    if bookmarks_path.exists() {
        fs::read_to_string(&bookmarks_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        fs::read_to_string("bookmarks.json").unwrap_or_else(|_| {
            r#"{"YouTube":"https://youtube.com","Google":"https://google.com","Wikipedia":"https://wikipedia.org","GitHub":"https://github.com"}"#.to_string()
        })
    }
}
