// ipc.rs — IPC token generation and message validation
use rand::RngCore;

/// Generates a cryptographically random 16-byte token as a hex string.
/// Used to prefix all IPC messages from trusted Aurora pages so that
/// arbitrary web pages cannot call internal browser commands.
pub fn generate_ipc_token() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Checks that `msg` starts with `token_prefix` and returns the payload after it.
/// Returns None if the token doesn't match — the message should be ignored.
pub fn validate_ipc_message<'a>(msg: &'a str, token_prefix: &str) -> Option<&'a str> {
    if msg.starts_with(token_prefix) {
        Some(&msg[token_prefix.len()..])
    } else {
        None
    }
}
