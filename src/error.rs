// error.rs — custom error page for network/navigation failures
use base64::Engine;

const BRAND_SVG: &str = include_str!("../brand.svg");

pub struct ErrorInfo {
    pub url: String,
    pub title: String,
    pub reason: String,
    pub code: String,
}

impl Default for ErrorInfo {
    fn default() -> Self {
        Self {
            url: String::new(),
            title: "Can't reach this page".to_string(),
            reason: "The site might be temporarily unavailable,<br>or you may have lost your internet connection.".to_string(),
            code: String::new(),
        }
    }
}

pub fn get_error_html(ipc_token: &str, info: &ErrorInfo) -> String {
    let svg_b64 = base64::engine::general_purpose::STANDARD.encode(BRAND_SVG.as_bytes());
    let icon_data_uri = format!("data:image/svg+xml;base64,{}", svg_b64);
    let error_json = serde_json::json!({
        "url": info.url,
        "title": info.title,
        "reason": info.reason,
        "code": info.code,
    })
    .to_string();
    let template = include_str!("error.html");
    template
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__ERROR_JSON__", &error_json)
        .replace("__ICON_BASE64__", &icon_data_uri)
}
