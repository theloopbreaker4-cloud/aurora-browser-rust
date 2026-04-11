// toolbar.rs — builds toolbar HTML with injected locale, config and bookmarks
use crate::config;

pub fn get_toolbar_html() -> String {
    let locale_json = config::load_locale();
    let config_json = config::load_config();
    let bookmarks_json = config::load_bookmarks();

    let base_html = include_str!("toolbar.html");

    let inject = format!(
        "<script>var AURORA_LOCALE={};var AURORA_CONFIG={};var AURORA_BOOKMARKS={};</script></head>",
        locale_json, config_json, bookmarks_json
    );
    base_html.replace("</head>", &inject)
}
