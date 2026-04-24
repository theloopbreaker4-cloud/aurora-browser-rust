// tab_groups.rs — stub aurora://tab_groups explainer (color-coded grouping coming soon)
use crate::config;

pub fn get_tab_groups_html(ipc_token: &str) -> String {
    let locale = config::load_locale();
    include_str!("tab_groups.html")
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__LOCALE_JSON__", &locale)
}
