// incognito.rs — stub aurora://incognito explainer page (real ephemeral session pending)
use crate::config;

pub fn get_incognito_html(ipc_token: &str) -> String {
    let locale = config::load_locale();
    include_str!("incognito.html")
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__LOCALE_JSON__", &locale)
}
