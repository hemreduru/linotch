# linotch

A small pill on the edge of your screen: one ring per coding assistant showing how
much of its limit you have burned, and one for whatever is playing — which is also
the play/pause button.

<p align="center"><img src="docs/preview.png" alt="The notch, shut and with a card open" width="478"></p>
<p align="center"><sub>Shut, and with a ring hovered. Sample data.</sub></p>

Linux only, and Wayland-first: the notch is a real `wlr-layer-shell` surface, not a
window nudged into place. It signs in nowhere — every reading is borrowed from a
credential a tool on your machine already holds.

## What it shows

| Ring | Source | How |
|---|---|---|
| **Claude** | Claude Code's OAuth token in `~/.claude/.credentials.json` | `api.anthropic.com/api/oauth/usage` — the same session and weekly windows `/usage` reports |
| **Codex** | The ChatGPT session in `~/.codex/auth.json` | `chatgpt.com/backend-api/wham/usage` — 5-hour and weekly windows |
| **Media** | Any MPRIS player on the session bus | Track progress; click to play/pause |

A ring appears only when that tool is installed *and* signed in. The arc shows the
window closest to its limit — the one that will actually stop you. Hovering opens a
card beside it with every window, its own colour grade, and when it resets.

Each ring carries its provider's own mark; the media ring carries the player's own
application icon, and swaps to a play/pause control under the pointer, so the ring
says both what is playing and that clicking does something.

Nothing is ever invented: when a vendor stops answering, the last reading stays,
dimmed, and the card says why it is old. A rejected token says so, and says that
using the tool once will refresh it. A rate limit is waited out for exactly as long
as the vendor's own `Retry-After` asks — asking again early is what keeps a limit
alive instead of letting it lapse.

Media works with anything that speaks MPRIS — Spotify, VLC, mpv, Firefox, Chromium,
KDE's browser integration — because that is the interface the desktop's own media
controls use. No helper binary, no per-player code.

## Display servers

The one genuinely per-environment part lives behind a trait in
[`src/surface.rs`](src/surface.rs).

| Surface | Environments | Mechanism |
|---|---|---|
| `LayerShell` | KDE/KWin, Hyprland, sway, wayfire, river, labwc | `zwlr_layer_shell_v1`, Overlay layer |
| `X11Dock` | any X11 session — KDE X11, XFCE, i3, Cinnamon, MATE | `_NET_WM_WINDOW_TYPE_DOCK`, kept above, moved by coordinate |
| `Floating` | Wayland without layer-shell — GNOME/Mutter | a plain window; Mutter lets no client place itself, so this degrades honestly rather than failing silently |

Detection is automatic. To override it:

```bash
LINOTCH_SURFACE=x11 linotch
```

**Adding an environment**: write a struct, `impl Surface` for it, add one arm to
`detect()`. `prepare()` runs before the window is realized, `place()` after it is
shown and on every size change. Nothing else in the codebase needs to know.

## Install

Needs GTK 3, gtk-layer-shell and a Rust toolchain.

```bash
sudo pacman -S --needed gtk3 gtk-layer-shell rust     # Arch / CachyOS
# Debian/Ubuntu: libgtk-3-dev libgtk-layer-shell-dev
```

```bash
cargo build --release
install -Dm755 target/release/linotch ~/.local/bin/linotch
```

Start it with your session:

```bash
install -Dm644 linotch.desktop ~/.config/autostart/linotch.desktop
```

## Use

Hover a ring for its card. Click the media ring to play/pause. Right-click for
refresh and quit.

Usage is re-read every three minutes. That is deliberate: a limit window does not
move fast enough to be worth a per-minute poll, and the vendors answer a burst with
a Retry-After measured in half hours. "Refresh now" in the right-click menu skips
the wait.

```bash
linotch --check      # what each source reports, and why one is missing
linotch --help
```

| Variable | Default | |
|---|---|---|
| `LINOTCH_EDGE` | `right` | `right`, `left`, `top`, `bottom` |
| `LINOTCH_OFFSET` | `0.5` | position along that edge, `0.0`–`1.0` |
| `LINOTCH_SURFACE` | auto | `layer`, `x11`, `floating` |

## Adding a provider

Implement `Provider` in [`src/providers.rs`](src/providers.rs) and add it to `all()`.
`present()` must be cheap and offline — it decides whether a ring is drawn at all,
before any network call. Two rules the existing ones follow:

- **Borrow, never manage.** Read the credential the tool already wrote; never refresh
  it, never write it back. A 401 is not an error to fix here, it is the tool's job.
- **Never invent a number.** Return an error and let the last reading go stale rather
  than showing a zero that looks like a reading.

## Credits

Provider marks are from [lobe-icons](https://github.com/lobehub/lobe-icons) (MIT) —
see [`assets/NOTICE.md`](assets/NOTICE.md). They remain the trademarks of their
owners.

The idea, and the provider wire formats, come from
[vinzdg/codenotch](https://github.com/vinzdg/codenotch) (MIT) — a macOS app with a
Windows port. This is a separate Linux implementation, not a fork: the surface layer,
the drawing, the media ring and the provider code are written from scratch for
GTK/Wayland.

## License

MIT
