<div align="center">
  <img src="brand.svg" width="120" height="120" alt="Aurora Browser"/>
  <h1>Aurora Browser</h1>
  <p>A lightweight, fast browser built with Rust — WebView2 and Servo engine support.</p>

  ![Version](https://img.shields.io/badge/version-0.2.0-blue)
  ![Platform](https://img.shields.io/badge/platform-Windows-lightgrey)
  ![License](https://img.shields.io/badge/license-MIT-green)
</div>

## Features

- Dual rendering engine — **WebView2** (default) or **Servo** (`--engine=servo`)
- Tabbed browsing with keyboard shortcuts
- Bookmark bar with quick access
- New tab portal with aurora background animation
- Address bar with search integration
- Find on page (`Ctrl+F`)
- Zoom (`Ctrl+=` / `Ctrl+-`)
- Fullscreen mode (`F11`)
- Developer Tools (`F12`)
- History, Downloads, Bookmarks pages
- Settings: language, search engine, startup, privacy
- Localization: English, Russian
- Dark theme with aurora aesthetic

## Built With

- [Rust](https://www.rust-lang.org/)
- [wry](https://github.com/tauri-apps/wry) — Cross-platform WebView (WebView2 backend)
- [tao](https://github.com/tauri-apps/tao) — Cross-platform windowing
- [Servo](https://servo.org/) — Experimental Rust browser engine (optional)
- [serde](https://serde.rs/) — Serialization

## Requirements

- Windows 10/11
- [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) (for default engine)
- Rust 1.92+

## Build

```bash
# Clone
git clone https://github.com/theloopbreaker4-cloud/AuroraBrowserRust
cd AuroraBrowserRust

# Build (WebView2 engine)
cargo build --release

# Build with Servo engine support
cargo build --release --features servo-engine

# Run
./target/release/aurora.exe

# Run with Servo engine
./target/release/aurora.exe --engine=servo
```

## Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+T` | New tab |
| `Ctrl+W` | Close tab |
| `Ctrl+L` | Focus address bar |
| `Ctrl+F` | Find on page |
| `Ctrl+P` | Print |
| `Ctrl+=` | Zoom in |
| `Ctrl+-` | Zoom out |
| `Ctrl+0` | Reset zoom |
| `F11` | Fullscreen |
| `F12` | Developer tools |
| `Ctrl+1–9` | Switch to tab N |

## Configuration

`config.json` is created automatically on first run in the same directory as the executable.

`bookmarks.json` — add your bookmarks:
```json
{
  "YouTube": "https://youtube.com",
  "GitHub": "https://github.com"
}
```

## License

MIT
