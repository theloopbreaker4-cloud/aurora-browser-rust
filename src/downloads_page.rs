// downloads_page.rs — builds downloads page HTML; downloads stored in downloads.json
use crate::config;

pub fn get_downloads_html(ipc_token: &str) -> String {
    let downloads_json = load_downloads();
    let template = include_str!("downloads.html");
    template
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__DOWNLOADS_JSON__", &downloads_json)
}

/// Loads downloads.json from exe dir or cwd, returns empty array on failure.
pub fn load_downloads() -> String {
    let dir = config::exe_dir();
    std::fs::read_to_string(dir.join("downloads.json"))
        .or_else(|_| std::fs::read_to_string("downloads.json"))
        .unwrap_or_else(|_| "[]".to_string())
}
