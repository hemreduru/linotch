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

use gtk::gdk;
use gtk::prelude::*;

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

    pub fn name(self) -> &'static str {
        match self {
            Edge::Left => "left",
            Edge::Right => "right",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
        }
    }
}

pub trait Surface {
    /// Shown in `--check` and the menu, so a bug report says which path ran.
    fn name(&self) -> &'static str;

    /// Before the window is realized. Layer-shell setup must happen here.
    fn prepare(
        &self,
        _w: &gtk::ApplicationWindow,
        _mon: Option<&gdk::Monitor>,
        _edge: Edge,
        _offset: f64,
    ) {
    }

    /// Move to a different edge on a window that is already up — what dragging the
    /// notch across the screen needs. Separate from `prepare` because that one also
    /// does the once-only setup.
    fn set_edge(&self, _w: &gtk::ApplicationWindow, _edge: Edge) {}

    /// False when the compositor owns the window's position and monitor, so
    /// monitor changes are none of the notch's business.
    fn follows_monitors(&self) -> bool {
        true
    }

    /// Pin the surface to a specific monitor.
    fn set_monitor(&self, _w: &gtk::ApplicationWindow, _mon: &gdk::Monitor) {}

    /// After the window is shown, and on monitor changes. For servers that place by
    /// coordinate.
    ///
    /// `size` is passed in rather than read back from the window: right after a
    /// resize `GtkWindow::size` still reports the old one — GTK's default 200x200 on
    /// the first call — and placing against that puts the notch a hundred pixels
    /// short of the bezel.
    fn place(
        &self,
        _w: &gtk::ApplicationWindow,
        _mon: &MonitorRect,
        _edge: Edge,
        _offset: f64,
        _size: (i32, i32),
    ) {
    }
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

    fn set_monitor(&self, w: &gtk::ApplicationWindow, mon: &gdk::Monitor) {
        use gtk_layer_shell::LayerShell as _;
        w.set_monitor(mon);
    }

    /// Re-anchoring a surface that is already mapped is not reliably picked up —
    /// KWin reads the anchor when the layer surface is created, and a live change
    /// left the notch floating in the middle of the screen, attached to nothing.
    /// So the surface is re-created instead of nudged. Cheap, because this only
    /// happens when a drag ends on a different edge.
    fn set_edge(&self, w: &gtk::ApplicationWindow, edge: Edge) {
        use gtk_layer_shell::{Edge as LsEdge, LayerShell as _};
        let mapped = w.is_visible();
        if mapped {
            w.hide();
        }
        // Cleared first: moving from one edge to another must not leave the old
        // anchor set, or the surface stretches between the two.
        for e in [LsEdge::Left, LsEdge::Right, LsEdge::Top, LsEdge::Bottom] {
            w.set_anchor(e, false);
        }
        w.set_anchor(
            match edge {
                Edge::Left => LsEdge::Left,
                Edge::Right => LsEdge::Right,
                Edge::Top => LsEdge::Top,
                Edge::Bottom => LsEdge::Bottom,
            },
            true,
        );
        // The leading edge is anchored too, so the margin set in `place` means
        // "this far from the top/left" rather than being ignored.
        w.set_anchor(leading(edge), true);
        if mapped {
            w.show();
        }
    }

    fn prepare(
        &self,
        w: &gtk::ApplicationWindow,
        mon: Option<&gdk::Monitor>,
        edge: Edge,
        offset: f64,
    ) {
        use gtk_layer_shell::{KeyboardMode, Layer, LayerShell as _};
        w.init_layer_shell();
        w.set_namespace("linotch");
        w.set_layer(Layer::Overlay);
        // No keyboard focus: the notch is glanced at and clicked, never typed into.
        // Exclusive is what steals input from the focused window.
        w.set_keyboard_mode(KeyboardMode::None);
        // 0 = float over the desktop without reserving a strut. Reserving one would
        // shrink every maximised window by the notch's width, which is not the deal.
        w.set_exclusive_zone(0);
        if let Some(m) = mon {
            w.set_monitor(m);
        }
        self.set_edge(w, edge);
        let _ = offset; // applied in place(), once the window's own size is known
    }

    /// layer-shell has no coordinates, so `offset` becomes a margin — and a margin
    /// positions the window's *leading edge*, not its centre. Subtracting half the
    /// window keeps 0.5 meaning centred, which is what it looks like it means.
    fn place(
        &self,
        w: &gtk::ApplicationWindow,
        mon: &MonitorRect,
        edge: Edge,
        offset: f64,
        size: (i32, i32),
    ) {
        use gtk_layer_shell::LayerShell as _;
        let (span, own) = if edge.vertical() {
            (mon.height, size.1)
        } else {
            (mon.width, size.0)
        };
        let m = offset_to_margin(span, own, offset);
        w.set_layer_shell_margin(leading(edge), m);
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

    fn prepare(
        &self,
        w: &gtk::ApplicationWindow,
        _mon: Option<&gdk::Monitor>,
        _edge: Edge,
        _offset: f64,
    ) {
        w.set_type_hint(gdk::WindowTypeHint::Dock);
        w.set_keep_above(true);
        w.set_skip_taskbar_hint(true);
        w.set_skip_pager_hint(true);
        w.set_decorated(false);
        w.stick(); // present on every virtual desktop
    }

    fn place(
        &self,
        w: &gtk::ApplicationWindow,
        mon: &MonitorRect,
        edge: Edge,
        offset: f64,
        size: (i32, i32),
    ) {
        let (x, y) = window_global_position(mon, edge, offset, size);
        w.move_(x, y);
        // Re-asserted after the window is realized: set before that, there is no X
        // window to hang the hints on yet.
        w.set_keep_above(true);
        w.stick();
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

    fn follows_monitors(&self) -> bool {
        false
    }

    fn prepare(
        &self,
        w: &gtk::ApplicationWindow,
        _mon: Option<&gdk::Monitor>,
        _edge: Edge,
        _offset: f64,
    ) {
        w.set_decorated(false);
        w.set_keep_above(true); // honoured on X11, ignored by Mutter on Wayland
        w.set_skip_taskbar_hint(true);
    }
}

// ------------------------------------------------------------------ detect

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

pub fn monitor_connector(screen: &gdk::Screen, idx: i32) -> Option<String> {
    use glib::translate::{FromGlibPtrFull, ToGlibPtr};
    unsafe {
        let ptr = gdk::ffi::gdk_screen_get_monitor_plug_name(screen.to_glib_none().0, idx);
        if ptr.is_null() {
            None
        } else {
            let s: glib::GString = FromGlibPtrFull::from_glib_full(ptr);
            Some(s.to_string())
        }
    }
}

pub fn enumerate_monitors(
    display: &gdk::Display,
    screen: Option<&gdk::Screen>,
) -> Vec<(MonitorSpec, MonitorRect, gdk::Monitor)> {
    let n = display.n_monitors();
    let mut list = Vec::with_capacity(n as usize);
    for i in 0..n {
        if let Some(mon) = display.monitor(i) {
            let rect = mon.geometry();
            let m_rect = MonitorRect::new(rect.x(), rect.y(), rect.width(), rect.height());
            let plug = screen.and_then(|s| monitor_connector(s, i));
            let model = mon.model().map(|s| s.to_string());
            let is_primary = mon.is_primary();
            let spec = MonitorSpec {
                connector: plug,
                model,
                is_primary,
            };
            list.push((spec, m_rect, mon));
        }
    }
    list
}

// ---------------------------------------------------------------- multi-monitor pure math

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl MonitorRect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x as f64
            && px < (self.x + self.width) as f64
            && py >= self.y as f64
            && py < (self.y + self.height) as f64
    }

    pub fn distance_sq(&self, px: f64, py: f64) -> f64 {
        let cx = px.clamp(self.x as f64, (self.x + self.width) as f64);
        let cy = py.clamp(self.y as f64, (self.y + self.height) as f64);
        (px - cx).powi(2) + (py - cy).powi(2)
    }
}

pub fn offset_to_margin(span: i32, own: i32, offset: f64) -> i32 {
    let off = offset.clamp(0.0, 1.0);
    let m = (span as f64 * off).round() as i32 - own / 2;
    m.clamp(0, (span - own).max(0))
}

pub fn window_local_position(
    mon: &MonitorRect,
    edge: Edge,
    offset: f64,
    size: (i32, i32),
) -> (i32, i32) {
    let (ww, wh) = size;
    let (span, own) = if edge.vertical() {
        (mon.height, wh)
    } else {
        (mon.width, ww)
    };
    let margin = offset_to_margin(span, own, offset);
    match edge {
        Edge::Left => (0, margin),
        Edge::Right => (mon.width - ww, margin),
        Edge::Top => (margin, 0),
        Edge::Bottom => (margin, mon.height - wh),
    }
}

pub fn window_global_position(
    mon: &MonitorRect,
    edge: Edge,
    offset: f64,
    size: (i32, i32),
) -> (i32, i32) {
    let (lx, ly) = window_local_position(mon, edge, offset, size);
    (mon.x + lx, mon.y + ly)
}

pub fn monitor_for_point(monitors: &[MonitorRect], gx: f64, gy: f64, current_idx: usize) -> usize {
    if monitors.is_empty() {
        return 0;
    }
    // 1. Direct hit
    for (i, m) in monitors.iter().enumerate() {
        if m.contains(gx, gy) {
            return i;
        }
    }
    // 2. Nearest monitor, with bias to current
    monitors
        .iter()
        .enumerate()
        .min_by(|(idx_a, a), (idx_b, b)| {
            let mut da = a.distance_sq(gx, gy);
            let mut db = b.distance_sq(gx, gy);
            if *idx_a == current_idx {
                da *= 0.8;
            }
            if *idx_b == current_idx {
                db *= 0.8;
            }
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorSpec {
    pub connector: Option<String>,
    pub model: Option<String>,
    pub is_primary: bool,
}

impl MonitorSpec {
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim();
        if q.is_empty() {
            return false;
        }
        if let Some(c) = &self.connector
            && c.eq_ignore_ascii_case(q)
        {
            return true;
        }
        if let Some(m) = &self.model
            && m.eq_ignore_ascii_case(q)
        {
            return true;
        }
        false
    }

    pub fn identifier(&self, index: usize) -> String {
        if let Some(c) = &self.connector
            && !c.is_empty()
            && c != "Unknown"
        {
            return c.clone();
        }
        if let Some(m) = &self.model
            && !m.is_empty()
            && m != "Unknown"
        {
            return m.clone();
        }
        self.connector
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("monitor-{}", index))
    }
}

/// Whether a monitor's geometry differs enough from the one the notch was placed
/// against to need placing again. Layer-shell margins are monitor-local, so only
/// the size matters there; `compare_origin` adds x/y for servers that place by
/// global coordinate.
pub fn placement_changed(old: Option<MonitorRect>, new: MonitorRect, compare_origin: bool) -> bool {
    match old {
        None => true,
        Some(o) => {
            o.width != new.width
                || o.height != new.height
                || (compare_origin && (o.x != new.x || o.y != new.y))
        }
    }
}

pub fn select_monitor(monitors: &[MonitorSpec], saved: Option<&str>) -> usize {
    if monitors.is_empty() {
        return 0;
    }
    if let Some(name) = saved
        && let Some(idx) = monitors.iter().position(|m| m.matches(name))
    {
        return idx;
    }
    if let Some(idx) = monitors.iter().position(|m| m.is_primary) {
        return idx;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge_for;

    #[test]
    fn offset_margin_centers_on_different_spans() {
        // 1920 display, 200px notch, offset 0.5 (center) -> margin should be (1920 * 0.5) - (200 / 2) = 860
        assert_eq!(offset_to_margin(1920, 200, 0.5), 860);
        // 1280 display, 200px notch, offset 0.5 (center) -> margin should be (1280 * 0.5) - (200 / 2) = 540
        assert_eq!(offset_to_margin(1280, 200, 0.5), 540);
        // offset 0.0 -> lead at 0
        assert_eq!(offset_to_margin(1920, 200, 0.0), 0);
        // offset 1.0 -> right against edge (1920 - 200 = 1720)
        assert_eq!(offset_to_margin(1920, 200, 1.0), 1720);
        // Clamping out-of-bounds offsets
        assert_eq!(offset_to_margin(1920, 200, -0.5), 0);
        assert_eq!(offset_to_margin(1920, 200, 1.5), 1720);
        // Window larger than span clamps safely to 0 without underflow
        assert_eq!(offset_to_margin(100, 200, 0.5), 0);
    }

    #[test]
    fn monitor_selection_by_saved_name_and_fallback() {
        let specs = vec![
            MonitorSpec {
                connector: Some("HEADLESS-1".into()),
                model: Some("Virtual Display".into()),
                is_primary: false,
            },
            MonitorSpec {
                connector: Some("HEADLESS-2".into()),
                model: Some("Virtual Display".into()),
                is_primary: false,
            },
        ];

        // Matches connector name
        assert_eq!(select_monitor(&specs, Some("HEADLESS-2")), 1);
        assert_eq!(select_monitor(&specs, Some("HEADLESS-1")), 0);
        assert_eq!(select_monitor(&specs, Some("headless-2")), 1); // case-insensitive

        // Falls back to index 0 when no primary
        assert_eq!(select_monitor(&specs, Some("NONEXISTENT")), 0);
        assert_eq!(select_monitor(&specs, None), 0);

        // Primary fallback when saved is missing or disconnected
        let specs_with_primary = vec![
            MonitorSpec {
                connector: Some("eDP-1".into()),
                model: Some("Built-in Screen".into()),
                is_primary: false,
            },
            MonitorSpec {
                connector: Some("DP-1".into()),
                model: Some("DELL U2720Q".into()),
                is_primary: true,
            },
        ];
        // Saved name matches model
        assert_eq!(select_monitor(&specs_with_primary, Some("DELL U2720Q")), 1);
        // Missing saved name falls back to primary
        assert_eq!(select_monitor(&specs_with_primary, None), 1);
        // Disconnected saved monitor falls back to primary
        assert_eq!(select_monitor(&specs_with_primary, Some("HDMI-1")), 1);
    }

    #[test]
    fn multi_monitor_origins_and_drag_selection() {
        // Output 1: 1920x1080 at (0, 0)
        let m1 = MonitorRect::new(0, 0, 1920, 1080);
        // Output 2: 1280x1024 at (1920, 0)
        let m2 = MonitorRect::new(1920, 0, 1280, 1024);
        let monitors = [m1, m2];

        // Notch placed on m2 at Top, offset 0.5, size 200x40
        let (lx, ly) = window_local_position(&m2, Edge::Top, 0.5, (200, 40));
        assert_eq!(lx, 540); // 1280/2 - 100
        assert_eq!(ly, 0);

        let (gx, gy) = window_global_position(&m2, Edge::Top, 0.5, (200, 40));
        assert_eq!(gx, 1920 + 540); // 2460
        assert_eq!(gy, 0);

        // Pointer inside notch at (100, 20): global coord is (2560, 20)
        let (ptr_gx, ptr_gy) = (gx as f64 + 100.0, gy as f64 + 20.0);
        assert_eq!(monitor_for_point(&monitors, ptr_gx, ptr_gy, 1), 1);

        // Dragged to the left onto m1: global coord (1500, 20)
        let target_idx = monitor_for_point(&monitors, 1500.0, 20.0, 1);
        assert_eq!(target_idx, 0);

        // Coordinates local to m1
        let target_lx = 1500.0 - m1.x as f64;
        let target_ly = 20.0 - m1.y as f64;
        assert_eq!(target_lx, 1500.0);
        assert_eq!(target_ly, 20.0);

        // edge_for on m1 (1920x1080)
        assert_eq!(
            edge_for(
                target_lx,
                target_ly,
                m1.width as f64,
                m1.height as f64,
                Edge::Top
            ),
            Edge::Top
        );

        // Dragged near right edge of m1 (1900, 500)
        assert_eq!(
            edge_for(1900.0, 500.0, m1.width as f64, m1.height as f64, Edge::Top),
            Edge::Right
        );

        // Dragged into m2 near its left edge (1930 in global -> 10 on m2)
        let m2_idx = monitor_for_point(&monitors, 1930.0, 500.0, 0);
        assert_eq!(m2_idx, 1);
        let m2_lx = 1930.0 - m2.x as f64; // 10.0
        assert_eq!(
            edge_for(m2_lx, 500.0, m2.width as f64, m2.height as f64, Edge::Right),
            Edge::Left
        );
    }

    #[test]
    fn preferred_vs_current_monitor_resolution_and_recovery() {
        let h1 = MonitorSpec {
            connector: Some("HEADLESS-1".into()),
            model: None,
            is_primary: true,
        };
        let h2 = MonitorSpec {
            connector: Some("HEADLESS-2".into()),
            model: None,
            is_primary: false,
        };

        let preferred = Some("HEADLESS-2");

        // 1. Both outputs connected: preferred HEADLESS-2 wins over primary HEADLESS-1
        let both = vec![h1.clone(), h2.clone()];
        assert_eq!(select_monitor(&both, preferred), 1);

        // 2. HEADLESS-2 disabled/disconnected: falls back to remaining output (HEADLESS-1 at index 0)
        let only_h1 = vec![h1.clone()];
        assert_eq!(select_monitor(&only_h1, preferred), 0);

        // 3. HEADLESS-2 re-enabled: with preferred STILL "HEADLESS-2", notch returns to HEADLESS-2
        let reconnected = vec![h1.clone(), h2.clone()];
        assert_eq!(select_monitor(&reconnected, preferred), 1);

        // 4. No preferred monitor: primary wins
        assert_eq!(select_monitor(&both, None), 0);
    }

    #[test]
    fn placement_is_stale_only_when_it_matters() {
        let a = MonitorRect::new(0, 0, 1920, 1080);
        let moved = MonitorRect::new(1920, 0, 1920, 1080);
        let resized = MonitorRect::new(0, 0, 1280, 720);

        // Nothing placed yet.
        assert!(placement_changed(None, a, false));
        // Same output, same size: nothing to redo.
        assert!(!placement_changed(Some(a), a, false));
        assert!(!placement_changed(Some(a), a, true));
        // Layer-shell margins are monitor-local, so a new origin alone is harmless...
        assert!(!placement_changed(Some(a), moved, false));
        // ...but X11 places by global coordinate.
        assert!(placement_changed(Some(a), moved, true));
        // A new size matters everywhere.
        assert!(placement_changed(Some(a), resized, false));
        assert!(placement_changed(Some(a), resized, true));
    }
}
