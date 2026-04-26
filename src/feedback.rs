// feedback.rs — aurora://feedback page builder.
// Pre-fills diagnostic info (active engine, Aurora version, OS, last lines of
// servo_log.txt) so a bug report has reproduction context attached.

use crate::config;

pub fn get_feedback_html(ipc_token: &str) -> String {
    let locale = config::load_locale();
    let cfg = config::load_config();
    let diag = collect_diagnostics();
    include_str!("feedback.html")
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__LOCALE_JSON__", &locale)
        .replace("__CONFIG_JSON__", &cfg)
        .replace("__DIAG_JSON__", &diag)
}

fn collect_diagnostics() -> String {
    let aurora_version = env!("CARGO_PKG_VERSION");
    let engine = config::get_engine();
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    // Tail of servo_log.txt, if it exists. Cap at last 30 lines so the report
    // size stays sane.
    let log_tail = read_log_tail(30);

    let log_json = serde_json::Value::Array(
        log_tail
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
    );

    serde_json::json!({
        "aurora_version": aurora_version,
        "engine": engine,
        "os": format!("{}-{}", os, arch),
        "log_tail": log_json,
    })
    .to_string()
}

fn read_log_tail(n: usize) -> Vec<String> {
    let path = config::exe_dir().join("servo_log.txt");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].iter().map(|s| s.to_string()).collect()
}
