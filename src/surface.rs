//! How the notch gets pinned to a screen edge — the one part that is genuinely
//! per-display-server, kept behind a trait so a new environment is a new `impl`
//! and one line in [`detect`], not a rewrite.
//!
//! Shipped implementations:
//!
//! | Surface      | Where it applies                                    | Mechanism                        |
//! |--------------|-----------------------------------------------------|----------------------------------|
//! | `LayerShell` | KDE/KWin, Hyprland, sway, wayfire, river, labwc, …  | `zwlr_layer_shell_v1`, Overlay   |
//! | `X11Dock`    | any X11 session (KDE X11, XFCE, i3, Cinnamon, …)    | `_NET_WM_WINDOW_TYPE_DOCK` + move|
//! | `Floating`   | Wayland without layer-shell (GNOME/Mutter)           | plain window, honest degradation |
//!
//! Override the choice with `LINOTCH_SURFACE=layer|x11|floating`.
//!
//! ## Adding an environment
//!
//! Write a struct, implement [`Surface`], and add one arm to [`detect`]. `prepare`
//! runs before the window is realized (that is where layer-shell must be set up);
//! `place` runs after it is shown and again on every monitor change, which is where
//! a server that needs explicit coordinates does its work.

use gtk::prelude::*;
use gtk::gdk;

/// Which screen edge the notch hugs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn parse(s: &str) -> Option<Edge> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            "top" => Some(Edge::Top),
            "bottom" => Some(Edge::Bottom),
            _ => None,
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

pub trait Surface {
    /// Shown in `--check` and the right-click menu, so a bug report says which path ran.
    fn name(&self) -> &'static str;

    /// Before the window is realized. Layer-shell setup must happen here.
    fn prepare(&self, _w: &gtk::ApplicationWindow, _edge: Edge, _offset: f64) {}

    /// After the window is shown, and on monitor changes. For servers that place by
    /// coordinate. A compositor-anchored surface leaves this empty.
    fn place(&self, _w: &gtk::ApplicationWindow, _edge: Edge, _offset: f64) {}
}

// ---------------------------------------------------------------- layer shell

/// Wayland's own answer to "a panel that floats above everything". The compositor
/// does the placement, so there is nothing to recompute on resolution changes,
/// and nothing breaks when the user moves the window's monitor.
pub struct LayerShell;

impl Surface for LayerShell {
    fn name(&self) -> &'static str {
        "wlr-layer-shell"
    }

    fn prepare(&self, w: &gtk::ApplicationWindow, edge: Edge, offset: f64) {
        use gtk_layer_shell::{Edge as LsEdge, KeyboardMode, Layer, LayerShell as _};
        w.init_layer_shell();
        w.set_namespace("linotch");
        w.set_layer(Layer::Overlay);
        // No keyboard focus: the notch is glanced at and clicked, never typed into.
        // Exclusive is what steals input from the focused window.
        w.set_keyboard_mode(KeyboardMode::None);
        // 0 = float over the desktop without reserving a strut. Reserving one would
        // shrink every maximised window by the notch's width, which is not the deal.
        w.set_exclusive_zone(0);

        let (anchor, along) = match edge {
            Edge::Left => (LsEdge::Left, [LsEdge::Top, LsEdge::Bottom]),
            Edge::Right => (LsEdge::Right, [LsEdge::Top, LsEdge::Bottom]),
            Edge::Top => (LsEdge::Top, [LsEdge::Left, LsEdge::Right]),
            Edge::Bottom => (LsEdge::Bottom, [LsEdge::Left, LsEdge::Right]),
        };
        w.set_anchor(anchor, true);
        // Anchoring only the one edge leaves the compositor to centre us along the
        // other axis, which is what the offset then nudges.
        for e in along {
            w.set_anchor(e, false);
        }

        // The leading edge is anchored too, so the margin set in `place` means
        // "this far from the top/left" rather than being ignored.
        w.set_anchor(leading(edge), true);
        let _ = offset; // applied in place(), once the window's own size is known
    }

    /// layer-shell has no coordinates, so `offset` becomes a margin — and a margin
    /// positions the window's *leading edge*, not its centre. Subtracting half the
    /// window keeps 0.5 meaning centred, which is what it looks like it means.
    fn place(&self, w: &gtk::ApplicationWindow, edge: Edge, offset: f64) {
        use gtk_layer_shell::LayerShell as _;
        let Some(mon) = monitor_geometry(w) else { return };
        let (ww, wh) = w.size();
        let (span, own) = if edge.vertical() {
            (mon.height(), wh)
        } else {
            (mon.width(), ww)
        };
        let m = (span as f64 * offset.clamp(0.0, 1.0)) as i32 - own / 2;
        w.set_layer_shell_margin(leading(edge), m.clamp(0, (span - own).max(0)));
    }
}

/// The edge the offset margin is measured from.
fn leading(edge: Edge) -> gtk_layer_shell::Edge {
    if edge.vertical() {
        gtk_layer_shell::Edge::Top
    } else {
        gtk_layer_shell::Edge::Left
    }
}

// --------------------------------------------------------------------- X11

/// X11 has no layer-shell; a dock-type window that is kept above and sticky is the
/// equivalent, and unlike Wayland it can be moved to an absolute coordinate.
pub struct X11Dock;

impl Surface for X11Dock {
    fn name(&self) -> &'static str {
        "x11-dock"
    }

    fn prepare(&self, w: &gtk::ApplicationWindow, _edge: Edge, _offset: f64) {
        w.set_type_hint(gdk::WindowTypeHint::Dock);
        w.set_keep_above(true);
        w.set_skip_taskbar_hint(true);
        w.set_skip_pager_hint(true);
        w.set_decorated(false);
        w.stick(); // present on every virtual desktop
    }

    fn place(&self, w: &gtk::ApplicationWindow, edge: Edge, offset: f64) {
        let Some(g) = monitor_geometry(w) else { return };
        let (ww, wh) = w.size();
        let off = offset.clamp(0.0, 1.0);
        let (x, y) = match edge {
            Edge::Left => (g.x(), g.y() + ((g.height() - wh) as f64 * off) as i32),
            Edge::Right => (
                g.x() + g.width() - ww,
                g.y() + ((g.height() - wh) as f64 * off) as i32,
            ),
            Edge::Top => (g.x() + ((g.width() - ww) as f64 * off) as i32, g.y()),
            Edge::Bottom => (
                g.x() + ((g.width() - ww) as f64 * off) as i32,
                g.y() + g.height() - wh,
            ),
        };
        w.move_(x, y);
    }
}

// ----------------------------------------------------------------- fallback

/// Wayland without `zwlr_layer_shell_v1` — in practice GNOME. Nothing here can pin
/// a window: Mutter grants no client the ability to place itself, by design. The
/// notch still runs and still reads, it just lives as an ordinary always-on-top-less
/// window the user positions once. Said out loud rather than failing silently.
pub struct Floating;

impl Surface for Floating {
    fn name(&self) -> &'static str {
        "floating (no layer-shell)"
    }

    fn prepare(&self, w: &gtk::ApplicationWindow, _edge: Edge, _offset: f64) {
        w.set_decorated(false);
        w.set_keep_above(true); // honoured on X11, ignored by Mutter on Wayland
        w.set_skip_taskbar_hint(true);
    }
}

// ------------------------------------------------------------------ detect

fn monitor_geometry(w: &gtk::ApplicationWindow) -> Option<gdk::Rectangle> {
    let display = w.display();
    let mon = w
        .window()
        .and_then(|gw| display.monitor_at_window(&gw))
        .or_else(|| display.primary_monitor())
        .or_else(|| display.monitor(0))?;
    Some(mon.geometry())
}

/// True when the session is Wayland — GDK knows, and asking it is more reliable
/// than `WAYLAND_DISPLAY`, which survives into an XWayland child.
fn on_wayland() -> bool {
    gdk::Display::default()
        .map(|d| d.type_().name().contains("Wayland"))
        .unwrap_or(false)
}

/// Pick a surface for this session. `LINOTCH_SURFACE` wins, so a user on an
/// environment we guessed wrong about has a fix that needs no rebuild.
pub fn detect() -> Box<dyn Surface> {
    match std::env::var("LINOTCH_SURFACE").ok().as_deref() {
        Some("layer") => return Box::new(LayerShell),
        Some("x11") => return Box::new(X11Dock),
        Some("floating") => return Box::new(Floating),
        Some(other) => eprintln!("linotch: unknown LINOTCH_SURFACE={other:?}, autodetecting"),
        None => {}
    }
    if !on_wayland() {
        return Box::new(X11Dock);
    }
    if gtk_layer_shell::is_supported() {
        Box::new(LayerShell)
    } else {
        Box::new(Floating)
    }
}
