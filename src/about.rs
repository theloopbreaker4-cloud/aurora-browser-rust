// about.rs — builds about page HTML with version/build info
use base64::Engine;
const BRAND_SVG: &str = include_str!("../brand.svg");
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn get_about_html(ipc_token: &str) -> String {
    let svg_b64 = base64::engine::general_purpose::STANDARD.encode(BRAND_SVG.as_bytes());
    let icon_data_uri = format!("data:image/svg+xml;base64,{}", svg_b64);
    let platform = format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH);
    let build_date = option_env!("AURORA_BUILD_DATE").unwrap_or("Unknown");
    let template = include_str!("about.html");
    template
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__ICON_BASE64__", &icon_data_uri)
        .replace("__VERSION__", VERSION)
        .replace("__WRY_VERSION__", "0.46")
        .replace("__PLATFORM__", &platform)
        .replace("__BUILD_DATE__", build_date)
}
