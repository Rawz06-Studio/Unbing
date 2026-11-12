<div align="center">

<img src="assets/icon.svg" width="128" height="128" alt="Unbing logo">

# Unbing

**Bing's picture of the day as your wallpaper. No telemetry, no account, no tracking.**

Open it once, tick a box, forget about it. It lives in your menu bar.

[![CI](https://github.com/Rawz06-Studio/Unbing/actions/workflows/ci.yml/badge.svg)](https://github.com/Rawz06-Studio/Unbing/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Rawz06-Studio/Unbing?sort=semver)](https://github.com/Rawz06-Studio/Unbing/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)

</div>

---

> **Disclaimer** — Unbing is not affiliated with, endorsed or sponsored by Microsoft. “Bing” is a
> trademark of Microsoft Corporation. Pictures are © Bing and their respective authors (credited
> under each picture) and are provided **for personal use as a wallpaper only**: don't redistribute them.

## Why

The official Bing Wallpaper app ships a search box, a browser extension prompt, analytics and a
background service that phones home. Unbing does one thing: it sets the picture of the day as your
wallpaper. It's a single native binary that talks to `www.bing.com` for the pictures, and to GitHub
only to check for its own updates (which you can turn off).

## Features

- **Picture of the day** — downloaded, applied as your wallpaper and kept in `Pictures/Unbing`.
- **The last 8 days** — browse them in the sidebar, set any of them as wallpaper.
- **Market** — `fr-FR`, `en-US`, `en-GB`, `de-DE`, `ja-JP`… the picture differs per market.
- **4K** — optional UHD downloads for high-density screens (falls back to 1080p when a picture has no UHD version).
- **Menu bar / notification area** — a small icon next to the clock (macOS menu bar, Windows notification area) opens a compact window: today's picture, previous/next, set as wallpaper, daily change on/off. Closing the main window keeps Unbing there.
- **Daily change** — one checkbox registers Unbing to start with your session, icon only. It switches right after Bing publishes, at the market's own midnight.
- **Self-update** — Unbing checks GitHub releases on startup (and once a day while running) and updates itself in one click. Can be turned off.
- **Offline-proof** — on a network or API error the current wallpaper stays, the cache is never wiped and the app never crashes.
- **Themes** — Auto (follows the system), Light or Dark.

And that's deliberately all: no search box, no widget, no account.

## Privacy

| | |
| --- | --- |
| Servers contacted | `https://www.bing.com` only (HTTPS, rustls, redirects refused) |
| Requests | **2 per picture of the day**: the metadata (8 days at once), then the image. Between two Bing publications: none. Retried once on a network error. |
| Extra requests | Only when *you* ask: changing market, or opening an older picture that isn't on disk yet (downloaded once). |
| Update check | `api.github.com` (latest release) once at startup and once a day, plus `github.com` to download an update you accepted. Turn it off in Settings → Updates. |
| Collected data | None. No analytics, crash reporting, account or cookie. Logs stay on your machine. |

The sidebar shows how many requests were made today.

## Install

Grab the binary for your platform from the [latest release](https://github.com/Rawz06-Studio/Unbing/releases/latest):

| Platform | File |
| --- | --- |
| macOS (Apple Silicon & Intel) | `Unbing-macos-universal.zip` (app) |
| Linux x86_64 | `unbing-linux-x86_64` |
| Linux ARM64 | `unbing-linux-aarch64` |
| Windows x86_64 | `Unbing-Setup-x86_64.exe` (installer) or `Unbing-windows-x86_64.zip` (portable) |

Then tick **Change wallpaper every day** in the compact window (menu bar icon) or in **Settings** (<kbd>⌘</kbd> <kbd>,</kbd>).

<details>
<summary><b>macOS: “Unbing can't be opened”</b></summary>

Builds are not notarized by Apple. Move `Unbing.app` to **Applications** (the daily change needs a
stable location), then remove the quarantine flag once:

```bash
xattr -dr com.apple.quarantine /Applications/Unbing.app
```

The daily change is a LaunchAgent (`~/Library/LaunchAgents/studio.rawz06.unbing.plist`) that starts
`Unbing --background` at login: menu bar icon only, no window and no Dock icon. It makes no request
until Bing has published a new picture.
</details>

<details>
<summary><b>Windows: installer and SmartScreen</b></summary>

The installer needs no admin rights: it installs Unbing for the current user in
`%LOCALAPPDATA%\Programs\Unbing`, adds it to the Start menu and registers an uninstaller, which also
removes the startup entry. Settings in `%APPDATA%\unbing` and pictures in `Pictures\Unbing` are kept.

The daily change is a `Run` registry value that starts `Unbing.exe --background` at login: an icon in
the notification area, no window. It makes no request until Bing has published a new picture.

Builds are not code-signed yet, so SmartScreen may show “Windows protected your PC”: click
**More info** → **Run anyway**.
</details>

<details>
<summary><b>Linux</b></summary>

```bash
chmod +x unbing-linux-x86_64 && ./unbing-linux-x86_64
```

There is no tray icon on Linux. The daily change is an XDG autostart entry
(`~/.config/autostart/unbing.desktop`) running `unbing --daemon`: no window, no icon. Tiling
compositors that ignore XDG autostart need `exec unbing --daemon` in their config.

Wallpaper backends, detected in this order: Hyprland (`hyprctl hyprpaper`), Sway (`swaymsg`),
KDE Plasma (`plasma-apply-wallpaperimage`), GNOME and derivatives (`gsettings`), other Wayland
compositors (`swaybg`), X11 (`feh`). It can be forced in Settings. Without any, the daily change is disabled.
</details>

### From source

Requires a recent [Rust toolchain](https://rustup.rs).

```bash
git clone https://github.com/Rawz06-Studio/Unbing.git
cd Unbing
cargo run --release
```

On Linux you may need: `libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.

## Usage

| Shortcut | Action |
| --- | --- |
| <kbd>↑</kbd> / <kbd>↓</kbd> | Newer / older picture |
| <kbd>⌘</kbd> <kbd>↵</kbd> | Set as wallpaper |
| <kbd>⌘</kbd> <kbd>,</kbd> | Settings |

Every action is also in the menus: the native menu bar on macOS (**Unbing**, **Picture**, **Window**,
**Help**), and a **File · Picture · Help** bar at the top of the window on Windows and Linux.
Settings and About open in their own windows. The menu bar / tray icon: left click opens the compact
window, right click shows **Open Unbing**, **Use Today's Picture** and **Quit Unbing**.
On Linux and Windows, use <kbd>Ctrl</kbd> instead of <kbd>⌘</kbd>.

### Command line

| Command | Effect |
| --- | --- |
| `unbing` | Opens the window (or shows it, if Unbing already runs in the background) |
| `unbing --background` | Starts with the menu bar / tray icon only (macOS, Windows) |
| `unbing --apply` | One headless pass: applies the picture of the day, then exits |
| `unbing --daemon` | No window, no icon: changes the picture at each Bing publication (single instance; used on Linux) |
| `--verbose` | Detailed logs on stderr (`UNBING_LOG` also accepts a `tracing` filter) |

## Storage

| | Settings and index | Pictures |
| --- | --- | --- |
| macOS | `~/Library/Application Support/unbing/` | `~/Pictures/Unbing/` |
| Linux | `~/.config/unbing/` | `$XDG_PICTURES_DIR/Unbing/` |
| Windows | `%APPDATA%\unbing\` | `%USERPROFILE%\Pictures\Unbing\` |

`config.toml` is plain TOML:

```toml
market = "fr-FR"
uhd = false
history_limit = 30      # pictures kept on disk; the current wallpaper is never deleted
auto_apply = true
linux_backend = "auto"
theme = "auto"
check_updates = true
# images_dir = "/path/to/folder"
```

Unbing only ever deletes pictures it downloaded itself: files you add to the folder are left alone.

## Releasing

Releases are made by GitHub Actions only — never by hand:

- **Every push to `main` that passes CI** publishes a new patch release (0.1.0 → 0.1.1).
- **Actions → Release → Run workflow** publishes a `minor` (→ 0.2.0) or `major` (→ 1.0.0) release.

Each release bumps the version in `Cargo.toml` and `Cargo.lock`, commits `Release vX.Y.Z` to `main`,
tags it, builds macOS (universal `.app`), Linux (x86_64, ARM64) and Windows (installer + portable zip),
and publishes the executables with checksums and generated notes. Changes to Markdown files alone
don't trigger a release.

Installed copies of Unbing pick it up on their next update check (unless turned off in Settings).

## Icon

The logo lives in [`assets/icon.svg`](assets/icon.svg). After editing it, regenerate the PNG, `.icns` and `.ico` files:

```bash
scripts/icons.sh
```

## Built with

[Rust](https://www.rust-lang.org) · [egui](https://github.com/emilk/egui) · [ureq](https://github.com/algesten/ureq)

## License

Code: [MIT](LICENSE) © Rawz06 Studio. The downloaded pictures are not covered by this license (see the disclaimer above).
