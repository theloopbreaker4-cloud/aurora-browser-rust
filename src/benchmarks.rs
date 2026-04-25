// benchmarks.rs — aurora://benchmarks landing page with one-click links to
// external compliance & performance test suites (html5test, WPT, Speedometer,
// MotionMark, etc). Use this to spot what aurora://test (feature surface
// only) cannot catch — actual rendering and runtime correctness.
use crate::config;

pub fn get_benchmarks_html(ipc_token: &str) -> String {
    let locale = config::load_locale();
    include_str!("benchmarks.html")
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__LOCALE_JSON__", &locale)
}
