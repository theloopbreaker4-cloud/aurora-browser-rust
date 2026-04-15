// portal.rs — builds new tab portal HTML with bookmarks and aurora SVG logo
use crate::config;
use base64::Engine;

const BRAND_SVG: &str = include_str!("../brand.svg");

pub fn get_portal_html(ipc_token: &str) -> String {
    let bookmarks_json = config::load_bookmarks();
    let svg_b64 = base64::engine::general_purpose::STANDARD.encode(BRAND_SVG.as_bytes());
    let icon_data_uri = format!("data:image/svg+xml;base64,{}", svg_b64);
    let template = include_str!("portal.html");
    template
        .replace("__BOOKMARKS_JSON__", &bookmarks_json)
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__ICON_BASE64__", &icon_data_uri)
}
