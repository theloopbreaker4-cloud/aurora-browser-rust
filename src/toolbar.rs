// toolbar.rs — builds toolbar HTML with injected locale, config and bookmarks
use crate::config;
use base64::Engine;

const BRAND_SVG: &str = include_str!("../brand.svg");

pub fn get_toolbar_html() -> String {
    let locale_json = config::load_locale();
    let config_json = config::load_config();
    let bookmarks_json = config::load_bookmarks();
    // Encode SVG as data URI for use as favicon in tabs
    let svg_b64 = base64::engine::general_purpose::STANDARD.encode(BRAND_SVG.as_bytes());
    let favicon = format!("data:image/svg+xml;base64,{}", svg_b64);

    let base_html = include_str!("toolbar.html");

    let inject = format!(
        "<script>var AURORA_LOCALE={};var AURORA_CONFIG={};var AURORA_BOOKMARKS={};var AURORA_FAVICON='{}';</script></head>",
        locale_json, config_json, bookmarks_json, favicon
    );
    base_html.replace("</head>", &inject)
}
