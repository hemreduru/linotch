<div align="center">

# linotch

**A usage notch for Linux.** One ring per AI coding assistant showing how much of
your limit is gone, plus one for whatever is playing — pinned to the edge of your
screen, on Wayland or X11.

[![CI](https://github.com/hemreduru/linotch/actions/workflows/ci.yml/badge.svg)](https://github.com/hemreduru/linotch/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
![Platform: Linux](https://img.shields.io/badge/platform-Linux-informational)
![Wayland + X11](https://img.shields.io/badge/Wayland-%2B%20X11-blue)
![Rust](https://img.shields.io/badge/built%20with-Rust-orange)

<img src="docs/preview.png" alt="linotch on the right edge of the screen: rings for Claude and Codex usage and a media player, a hover card showing limit windows, and the right-click menu" width="731">

<sub>Shut, with a ring hovered, and with the menu open. Sample data.</sub>

</div>

---

## What it is

linotch is a small always-on-top panel — a "notch" — that answers one question at a
glance: **how much of my AI coding assistant's usage limit is left?** It reads
Claude Code and Codex usage directly from the vendors' own endpoints, using the
credentials those tools already keep on your machine, and draws one colour-graded
ring per account.

It also shows what is playing, via MPRIS, and the media ring doubles as a
play/pause button.

It signs in nowhere, stores no secret, sends nothing anywhere, and has no telemetry.

## Features

- **Claude Code usage** — session and weekly limits, from `api.anthropic.com`, the
  same windows `/usage` reports.
- **Codex / ChatGPT usage** — 5-hour and weekly windows from the ChatGPT backend.
- **Media control** — any MPRIS player (Spotify, VLC, mpv, Firefox, Chromium, KDE's
  browser integration). Track progress on the ring, click to play/pause.
- **Wayland-native** — a real `wlr-layer-shell` surface, not a window nudged into
  place. Falls back to an X11 dock where layer-shell is missing.
- **Drag it anywhere** — press and drag; it snaps to whichever screen edge you take
  it to and remembers where you left it.
- **Honest about failure** — a stale reading stays dimmed and says why; a rate limit
  is waited out for exactly as long as the vendor's `Retry-After` asks.
- **Light** — a ~4 MB binary, no daemon, no web view, no Electron.

## Install

### Arch Linux / CachyOS / EndeavourOS

```bash
sudo pacman -S --needed gtk3 gtk-layer-shell rust
git clone https://github.com/hemreduru/linotch && cd linotch
cargo build --release
install -Dm755 target/release/linotch ~/.local/bin/linotch
sed "s|^Exec=linotch|Exec=$HOME/.local/bin/linotch|" linotch.desktop \
  > ~/.config/autostart/linotch.desktop      # start with your session
linotch &
```

A [`PKGBUILD`](PKGBUILD) is included if you would rather `makepkg -si`.

### Debian / Ubuntu / Pop!_OS / Mint

```bash
sudo apt install -y libgtk-3-dev libgtk-layer-shell-dev cargo
git clone https://github.com/hemreduru/linotch && cd linotch
cargo build --release
install -Dm755 target/release/linotch ~/.local/bin/linotch
sed "s|^Exec=linotch|Exec=$HOME/.local/bin/linotch|" linotch.desktop \
  > ~/.config/autostart/linotch.desktop      # start with your session
linotch &
```

### Fedora / Nobara

```bash
sudo dnf install -y gtk3-devel gtk-layer-shell-devel cargo
git clone https://github.com/hemreduru/linotch && cd linotch
cargo build --release
install -Dm755 target/release/linotch ~/.local/bin/linotch
```

### Prebuilt binary

Each [release](https://github.com/hemreduru/linotch/releases) ships an
`x86_64-linux` tarball with a `sha256`. You still need `gtk3` and `gtk-layer-shell`
installed — every distro has both.

## Desktop support

| Desktop / compositor | Session | How it attaches | Status |
|---|---|---|---|
| **KDE Plasma** | Wayland | `wlr-layer-shell`, overlay layer | ✅ tested |
| **KDE Plasma** | X11 / XWayland | `_NET_WM_WINDOW_TYPE_DOCK` | ✅ tested |
| **Hyprland, sway, river, wayfire, labwc** | Wayland | `wlr-layer-shell` | ✅ supported |
| **GNOME** | Wayland | auto-restarts under XWayland, then X11 dock | ✅ supported |
| **XFCE, i3, Cinnamon, MATE, LXQt** | X11 | `_NET_WM_WINDOW_TYPE_DOCK` | ✅ supported |

**On GNOME**, Mutter implements no layer-shell and lets no Wayland client place
itself. linotch detects that and re-executes itself under XWayland, where an
ordinary dock window works properly. Nothing to configure. Set `LINOTCH_NO_REEXEC=1`
if you would rather it did not.

## Usage

Hover a ring to open its card. Click the media ring to play/pause. Right-click the
body for the menu. Press and drag the body to move it.

```bash
linotch            # run it
linotch --check    # what each source reports, and why one is missing
linotch --help
```

| Variable | Default | |
|---|---|---|
| `LINOTCH_EDGE` | `right` | `right`, `left`, `top`, `bottom` |
| `LINOTCH_OFFSET` | `0.5` | position along that edge, `0.0`–`1.0` |
| `LINOTCH_OPACITY` | `1.0` | panel opacity, `0.35`–`1.0` |
| `LINOTCH_SURFACE` | auto | `layer`, `x11`, `floating` |
| `LINOTCH_NO_REEXEC` | — | stay on Wayland even without layer-shell |

Where you drag it is saved to `~/.config/linotch/config.json`. The environment
variables win when set, so a one-off override never overwrites what you placed.

<div align="center">
<img src="docs/live.png" alt="linotch running on KDE Plasma Wayland, showing a Claude usage ring at 19%" width="180">
<br><sub>Running on KDE Plasma (Wayland).</sub>
</div>

## FAQ

**Does it need my API key?**
No. It never asks for one and never stores a credential. It reads the token Claude
Code already wrote to `~/.claude/.credentials.json`, and the ChatGPT session Codex
keeps in `~/.codex/auth.json`, and asks each vendor's own usage endpoint. Nothing
leaves your machine except those two requests, to vendors you are already signed in
to.

**The Claude ring says "token expired".**
`~/.claude/.credentials.json` is written by the Claude Code **CLI**. If you only use
Claude Code inside the desktop app, that file is never refreshed and its token goes
stale. Run `claude` in a terminal once and the ring fills in. linotch deliberately
does not refresh it itself — it borrows credentials, it does not manage them.

**A ring is missing.**
Run `linotch --check`; it prints, per source, whether the tool is installed, signed
in, rate limited, or answering fine. A ring is drawn only for a tool that is both
installed and signed in.

**Does it work on GNOME?**
Yes — see the table above. It restarts itself under XWayland automatically, because
Mutter supports no protocol that would let a client place itself on Wayland.

**Does it work with multiple monitors?**
It attaches to the monitor the compositor puts it on and follows that monitor's
geometry. Drag it to move it.

**How often does it poll?**
Every three minutes, and it honours `Retry-After` on a rate limit. A limit window
does not move fast enough to be worth a per-minute poll. "Refresh now" in the
right-click menu skips the wait.

**Does it support Cursor / Copilot / Gemini / OpenCode?**
Not yet. The provider interface is one trait with four methods — see
[Adding a provider](#adding-a-provider). PRs welcome.

**Why not a Waybar or Polybar module?**
Those are bars; this is a notch that opens a card, controls media, and moves where
you drag it. If you already live in Waybar, its custom-module support may suit you
better — `linotch --check` output is easy to parse.

## Adding a provider

Implement `Provider` in [`src/providers.rs`](src/providers.rs) and add it to `all()`:

```rust
impl Provider for MyTool {
    fn label(&self) -> &'static str { "MyTool" }
    fn asset(&self) -> &'static str { "mytool" }   // assets/mytool.png
    fn present(&self) -> bool { /* cheap, offline */ }
    fn read(&self) -> Result<Vec<Window>, Error> { /* one HTTP call */ }
}
```

Two rules the existing ones follow:

- **Borrow, never manage.** Read the credential the tool already wrote; never
  refresh it, never write it back. A 401 is not an error to fix here.
- **Never invent a number.** Return an error and let the last reading go stale
  rather than showing a zero that looks like a reading.

## Adding a desktop environment

The one genuinely per-display-server part lives behind a trait in
[`src/surface.rs`](src/surface.rs): write a struct, `impl Surface`, add one arm to
`detect()`. `prepare()` runs before the window is realized, `set_edge()` moves it
between edges, `place()` positions it along one. Nothing else in the codebase needs
to know.

## Design

The look is [codenotch](https://github.com/vinzdg/codenotch)'s, taken from its
source rather than eyeballed from screenshots. Upstream measured every distance off
a 2000×2000 design frame and anchored the scale on one value — the provider ring is
44 pt across and 117 px in the frame — so [`src/draw.rs`](src/draw.rs) reproduces
the same ratios from the same numbers, and the palette (`#00FF88` / `#F2FF00` /
`#FF3F00`, `#303030` track, `#808080` secondary text) is upstream's sampled values.
One number differs: the whole scale is multiplied by 0.85, because upstream is sized
to sit in a Mac's menu bar.

That includes the parts that carry the character: the inverse-rounded flares where
the body meets the bezel, the thin progress arc riding down the middle of the thick
track, the percentage under each ring, and the tail on the hover card.

Provider marks are drawn in their own brand colours; the media ring takes its colour
from the player's application icon, weighted by saturation so it picks up what the
eye does rather than averaging to grey.

## Building and contributing

```bash
cargo test                        # geometry, parsing, backoff, edge snapping
cargo test -- --ignored preview   # renders docs/preview.png
cargo clippy --all-targets
```

CI runs `fmt`, `clippy -D warnings`, the tests and a release build on every push.
Issues and PRs welcome — especially new providers and new desktop environments.

## Credits

- Design and provider wire formats: [vinzdg/codenotch](https://github.com/vinzdg/codenotch) (MIT) — a macOS app with a Windows port. linotch is a separate Linux implementation, not a fork; no code is shared.
- Provider marks: [lobe-icons](https://github.com/lobehub/lobe-icons) (MIT) — see [`assets/NOTICE.md`](assets/NOTICE.md). The marks remain the trademarks of their owners.

## License

MIT — see [LICENSE](LICENSE).

---

<sub>Keywords: Claude Code usage monitor for Linux · Codex usage tracker · AI coding
assistant limit widget · Wayland layer-shell panel · KDE Plasma notch · GNOME dock
widget · MPRIS media controls · Rust GTK desktop applet</sub>
