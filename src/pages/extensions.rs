// extensions.rs — stub aurora://extensions page (extension manager + store, coming soon)
use crate::config;

pub fn get_extensions_html(ipc_token: &str) -> String {
    let locale = config::load_locale();
    include_str!("extensions.html")
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__LOCALE_JSON__", &locale)
}
