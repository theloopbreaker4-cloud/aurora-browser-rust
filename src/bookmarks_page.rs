// bookmarks_page.rs — full-page bookmarks manager (aurora://bookmarks)
use crate::config;

pub fn get_bookmarks_html(ipc_token: &str) -> String {
    let bookmarks_json = config::load_bookmarks();
    let template = include_str!("bookmarks.html");
    template
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__BOOKMARKS_JSON__", &bookmarks_json)
}
