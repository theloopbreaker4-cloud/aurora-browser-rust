# Contributing to Aurora Browser

Welcome! Aurora is an open-source browser built with Rust. We appreciate all contributions.

## Ways to Contribute

- Bug reports and feature requests via [Issues](../../issues)
- Code contributions via [Pull Requests](../../pulls)
- Improving documentation
- Adding localizations (new languages in `locales/`)

## Getting Started

1. **Fork** the repository
2. **Clone** your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/AuroraBrowserRust
   cd AuroraBrowserRust
   ```
3. **Create a branch** for your change:
   ```bash
   git checkout -b feature/your-feature-name
   ```
4. **Build** to make sure everything works:
   ```bash
   cargo build
   ```
5. **Make your changes**
6. **Commit** with a clear message:
   ```bash
   git commit -m "Add: short description of what you did"
   ```
7. **Push** and open a Pull Request

## Code Style

- Standard Rust formatting: `cargo fmt` before committing
- No warnings: `cargo clippy` should pass
- Keep it simple — Aurora is intentionally minimal

## Project Structure

```
src/
  main.rs      — entry point
  app.rs       — event loop, window setup
  webviews.rs  — WebView creation and IPC routing
  events.rs    — UserEvent enum (IPC commands)
  toolbar.rs   — toolbar HTML builder
  portal.rs    — new tab portal HTML builder
  settings.rs  — settings page HTML builder
  config.rs    — config/bookmarks/locale loading
  ipc.rs       — IPC token generation and validation
  icon.rs      — window icon loader
  toolbar.html — toolbar UI (tabs, nav bar, bookmark bar)
  portal.html  — new tab page
  settings.html — settings page
locales/
  en.json      — English strings
  ru.json      — Russian strings
```

## Adding a Localization

1. Copy `locales/en.json` to `locales/XX.json` (e.g. `de.json`)
2. Translate the values (keep the keys unchanged)
3. Add the language option in `src/settings.html` → `langSelect`
4. Open a PR

## Reporting Bugs

Please include:
- OS version (Windows 10/11)
- Steps to reproduce
- What you expected vs what happened

## Questions

Open an issue with the `question` label — we are happy to help.
