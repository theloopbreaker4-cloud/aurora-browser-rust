# Aurora Browser

A lightweight, fast browser built with Rust.

![Aurora Browser](icon.png)

## Features

- Tabbed browsing with keyboard shortcuts
- Bookmark bar with quick access
- New tab portal with aurora background animation
- Address bar with search integration
- Find on page (Ctrl+F)
- Zoom (Ctrl+= / Ctrl+-)
- Fullscreen mode (F11)
- Developer Tools (F12)
- Settings: language, search engine, startup, privacy
- Localization: English, Russian
- Dark theme with aurora aesthetic

## Built With

- [Rust](https://www.rust-lang.org/)
- [wry](https://github.com/tauri-apps/wry) — Cross-platform WebView
- [tao](https://github.com/tauri-apps/tao) — Cross-platform windowing
- [serde](https://serde.rs/) — Serialization

## Requirements

- Windows 10/11 with [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
- Rust 1.75+

## Build

```bash
# Clone
git clone https://github.com/YOUR_USERNAME/AuroraBrowserRust
cd AuroraBrowserRust

# Generate icon (one-time)
rustc gen_icon.rs -o gen_icon.exe && gen_icon.exe

# Build
cargo build --release

# Run
./target/release/aurora.exe
```

## Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+T | New tab |
| Ctrl+W | Close tab |
| Ctrl+L | Focus address bar |
| Ctrl+F | Find on page |
| Ctrl+P | Print |
| Ctrl+= | Zoom in |
| Ctrl+- | Zoom out |
| Ctrl+0 | Reset zoom |
| F11 | Fullscreen |
| F12 | Developer tools |
| Ctrl+1-9 | Switch to tab N |

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
