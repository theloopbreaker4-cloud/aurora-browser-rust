// internal_server.rs — Tiny HTTP server bound to 127.0.0.1 that serves Aurora's
// internal aurora:// pages. Why: Servo (correctly per spec) refuses
// localStorage / IndexedDB / SecureContext APIs on opaque origins, and
// data:text/html;base64,... URLs always have an opaque origin. By serving the
// same HTML over http://127.0.0.1:<port>/ (a tuple origin), our internal pages
// get full Web platform access. The port is random and only bound to loopback
// so this server is not reachable from the network.
//
// Implementation note: stdlib TcpListener + hand-rolled HTTP/1.1 parsing keeps
// us free of network/runtime dependencies (no tokio, no hyper). We only handle
// GET, fixed routes, no concurrency beyond per-request thread spawn.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

/// Resolves a request path (e.g. "/test", "/settings") to (mime, body).
type RouteResolver = Arc<dyn Fn(&str) -> Option<(String, String)> + Send + Sync + 'static>;

/// Started server handle. The bound port lets the embedder rewrite
/// `aurora://X` URLs into `http://127.0.0.1:<port>/X`.
pub struct InternalServer {
    pub port: u16,
}

impl InternalServer {
    /// Bind to an ephemeral 127.0.0.1 port and start serving on a background thread.
    pub fn start(resolver: RouteResolver) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
        let port = listener.local_addr()?.port();

        thread::Builder::new()
            .name("aurora-internal-http".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let resolver = resolver.clone();
                    let _ = thread::Builder::new()
                        .name("aurora-internal-http-conn".into())
                        .spawn(move || {
                            handle_connection(stream, resolver);
                        });
                }
            })?;

        Ok(InternalServer { port })
    }
}

fn handle_connection(mut stream: TcpStream, resolver: RouteResolver) {
    let mut buf = [0u8; 8192];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let req = String::from_utf8_lossy(&buf[..n]);

    let first_line = req.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_path = parts.next().unwrap_or("/");

    if method != "GET" {
        write_status(&mut stream, 405, "Method Not Allowed", "text/plain", b"only GET");
        return;
    }

    let path = raw_path.split('?').next().unwrap_or("/");

    match resolver(path) {
        Some((mime, body)) => {
            write_status(&mut stream, 200, "OK", &mime, body.as_bytes());
        }
        None => {
            write_status(&mut stream, 404, "Not Found", "text/plain", b"404");
        }
    }
}

fn write_status(stream: &mut TcpStream, code: u16, phrase: &str, mime: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {code} {phrase}\r\n\
         Content-Type: {mime}; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        len = body.len(),
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

/// Build the route resolver closure that maps `/path` to a fully-rendered
/// aurora:// page. The IPC token is captured so pages reach the same Rust
/// handler whether they were loaded via wry load_html or via this server.
pub fn build_resolver(ipc_token: String) -> RouteResolver {
    Arc::new(move |path: &str| -> Option<(String, String)> {
        let html = match path.trim_end_matches('/') {
            "" | "/newtab" | "/portal" => crate::portal::get_portal_html(&ipc_token),
            "/settings" => crate::settings::get_settings_html(&ipc_token),
            "/history" => crate::history::get_history_html(&ipc_token),
            "/bookmarks" => crate::bookmarks_page::get_bookmarks_html(&ipc_token),
            "/downloads" => crate::downloads_page::get_downloads_html(&ipc_token),
            "/about" => crate::about::get_about_html(&ipc_token),
            "/test" => crate::test_page::get_test_html(),
            "/extensions" => crate::extensions::get_extensions_html(&ipc_token),
            "/incognito" => crate::incognito::get_incognito_html(&ipc_token),
            "/tab_groups" => crate::tab_groups::get_tab_groups_html(&ipc_token),
            _ => return None,
        };
        Some(("text/html".to_string(), html))
    })
}

/// Convenience: start the server with the standard route table.
pub fn start_with_default_routes(ipc_token: &str) -> std::io::Result<InternalServer> {
    InternalServer::start(build_resolver(ipc_token.to_string()))
}
