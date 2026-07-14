# D2H — Directory to HTML

**Turn any directory tree into a fast, searchable, shareable HTML catalog.**

D2H scans a folder and generates a self-contained catalog of its structure —
file names, sizes and dates (no file contents). Send it as a single HTML file,
or upload the web package to any static hosting and share a link. Built for
huge trees: 155,000 files across 6,000 folders (576 GB of data) catalog in
about 3 seconds on a modern laptop, producing a 5 MB web package or a single
7 MB HTML file.

Originally built for a data recovery lab, where customers need to review
what was recovered before the data ships — useful anywhere you need to show
someone *what's in there* without sending the data itself.

## Features

- **Two output modes**
  - *Single HTML file* — everything embedded (gzip + base64), works offline,
    small enough to e-mail
  - *Web package* (`.tar.gz`) — `index.html` + compressed data chunks loaded
    on demand; upload to any static host and send a link
- **Fast** — native Rust scanner, no indexing service, no database
- **Full-text search** — wildcards, accent-insensitive, path search,
  per-folder scope
- **Statistics** — file type categories, pie charts, per-folder totals
- **Fully responsive** — the generated catalog works on phones and tablets
- **7 catalog languages** — English, Czech, German, Italian, Spanish,
  French, Polish; GUI in the same 7 languages with automatic OS detection
- **Custom branding** — your logo and header colors via a JSON config or
  the in-app Customize dialog
- **Cross-platform** — Windows, macOS (Apple Silicon + Intel), Linux
- **GUI + CLI** — drag & drop desktop app, plus a scriptable command-line
  binary for automation
- **Privacy** — catalogs contain metadata only (names, sizes, dates), never
  file contents; no account, no telemetry

## Download

Grab the latest build for your platform from
[Releases](https://github.com/smikes75/d2h/releases/latest):

| Platform | File |
|---|---|
| Windows portable (no install) | `D2H-portable-windows-x64.exe` |
| Windows installer (bundled WebView2) | `D2H-setup-windows-x64.exe` |
| macOS Apple Silicon | `D2H-macos-apple-silicon.dmg` |
| macOS Intel | `D2H-macos-intel.dmg` |
| Linux AppImage | `D2H-linux-x86_64.AppImage` |
| Linux Debian/Ubuntu | `D2H-linux-amd64.deb` |

Notes: Windows may show a SmartScreen warning for the unsigned binary
("More info → Run anyway"). On macOS, right-click → Open on first launch
(the app is not notarized).

## Quick start

**GUI:** launch D2H, drop a folder onto the window, pick the output format
and hit Generate.

**CLI:**

```bash
# Web package (tar.gz ready for your server)
d2h --dir /data/case42 --output /srv/catalogs/case42

# Plus a single-file HTML for e-mail
d2h --dir /data/case42 --output /srv/catalogs/case42 --html --lang en
```

Run `d2h --help` for all options (hidden files, symlinks, empty dirs,
languages, themes, custom logo…).

## Configuration

All site-specific behavior lives in an optional `d2h.config.json`, searched in
this order:

1. `--config <path>` (CLI flag)
2. `d2h.config.json` next to the executable
3. `%APPDATA%\d2h\config.json` (Windows) / `~/.config/d2h/config.json`
   (macOS, Linux)

See [`d2h-rust/d2h.config.example.json`](d2h-rust/d2h.config.example.json)
for a commented example. You can set:

- **branding** — logo path, header background (color or gradient),
  attribution footer
- **defaults** — catalog language, theme, GUI language, output directories
- **jobId** — an optional regex pattern for order/case numbers in your
  workflow; matching rules can auto-select language, theme, output paths and
  branding per rule

Everything works out of the box with no config file.

## Building from source

Requires [Rust](https://rustup.rs/) and the
[Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for
your platform.

```bash
cd d2h-rust

# CLI binary
cargo build --release            # -> target/release/d2h

# Desktop app
cd src-tauri
cargo build --release            # -> target/release/d2h-gui

# Bundled installers (NSIS / .app / AppImage / .deb)
cargo tauri build
```

Tests: `cargo test` in `d2h-rust/`.

## Acknowledgements

D2H was originally inspired by
[Snap2HTML](https://www.rlvision.com/snap2html/about.php) (RL Vision) and
[LinuxDir2HTML](https://github.com/homeisfar/LinuxDir2HTML); the current
implementation is a from-scratch rewrite sharing no code with either. If you
need clickable `file://` links or CSV/JSON export, Snap2HTML remains a fine
choice.

## License

[MIT](LICENSE) — © 2026 [DataHelp](https://datahelp.eu)
