// history.rs — builds history page HTML; history entries stored in history.json
use crate::config;

pub fn get_history_html(ipc_token: &str) -> String {
    let history_json = load_history();
    let template = include_str!("history.html");
    template
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__HISTORY_JSON__", &history_json)
}

/// Loads history.json from exe dir, cwd fallback, or returns empty array.
pub fn load_history() -> String {
    let dir = config::exe_dir();
    std::fs::read_to_string(dir.join("history.json"))
        .or_else(|_| std::fs::read_to_string("history.json"))
        .unwrap_or_else(|_| "[]".to_string())
}

/// Truncates history.json to an empty list.
pub fn clear_history() {
    let dir = config::exe_dir();
    let _ = std::fs::write(dir.join("history.json"), "[]");
    let _ = std::fs::write("history.json", "[]");
}

/// Appends a visit entry to history.json.
/// Entry format: {"title": "...", "url": "...", "time": <unix_ms>}
pub fn push_history_entry(title: &str, url: &str) {
    // Skip internal aurora:// pages and data: URLs
    if url.starts_with("aurora://") || url.starts_with("data:") {
        return;
    }
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let existing = load_history();
    let mut entries: Vec<serde_json::Value> = serde_json::from_str(&existing).unwrap_or_default();
    entries.insert(
        0,
        serde_json::json!({
            "title": title,
            "url": url,
            "time": time,
        }),
    );
    // Keep at most 2000 entries
    entries.truncate(2000);
    if let Ok(json) = serde_json::to_string(&entries) {
        let dir = config::exe_dir();
        let _ = std::fs::write(dir.join("history.json"), &json);
        let _ = std::fs::write("history.json", &json);
    }
}
