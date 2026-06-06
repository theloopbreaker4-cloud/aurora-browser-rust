// settings.rs — builds settings page HTML with injected config and locale
use crate::config;

pub fn get_settings_html(ipc_token: &str) -> String {
    let config_json = config::load_config();
    let locale_json = config::load_locale();

    let template = include_str!("settings.html");
    template
        .replace("__CONFIG_JSON__", &config_json)
        .replace("__LOCALE_JSON__", &locale_json)
        .replace("__IPC_TOKEN__", ipc_token)
}
