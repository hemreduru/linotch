//! Cairo drawing, and the geometry that click handling shares with it.
//!
//! Every number here is codenotch's, converted from its design frame rather than
//! re-invented: upstream measured them off a 2000×2000 Figma export and anchored
//! the scale on one value (the provider ring is 44pt across and 117px in the
//! frame), so [`px`] reproduces the same proportions. The palette is likewise
//! upstream's sampled values, not approximations of them.
//!
//! The one thing deliberately *not* copied is the window model. codenotch draws
//! the card in a second window; linotch has one surface, and the card lives inside
//! it. That is why the window's size along the rail is computed without reference
//! to which ring is hovered — see [`layout`].

use crate::icons;
use crate::ring::{Glyph, Ring};
use crate::surface::Edge;
use gtk::pango;
use std::f64::consts::PI;

/// Points per pixel of codenotch's design frame: the ring is 44pt across and
/// measures 117px there — times 0.85, because upstream is sized for a Mac's menu
/// bar and this reads as too big on a desktop screen. One number, so everything
/// shrinks together and the proportions stay upstream's.
const SCALE: f64 = (44.0 / 117.0) * 0.85;

/// A distance measured in design-frame pixels.
const fn px(frame_px: f64) -> f64 {
    frame_px * SCALE
}

/// Cap-height fraction of an em, used by upstream to turn a measured cap height
/// back into a point size.
const CAP_RATIO: f64 = 0.714;
const fn font_px(cap_px: f64) -> f64 {
    px(cap_px) / CAP_RATIO
}

// ---- notch body -------------------------------------------------------------
const RAIL_DEPTH: f64 = px(186.0);
const CURL: f64 = px(103.0); // the inverse flare back out to the bezel
const CORNER: f64 = px(78.8);
const PAD_LEAD: f64 = px(69.5); // body start -> first ring
const PAD_TRAIL: f64 = px(50.1); // last label -> body end
const CELL_SPACING: f64 = px(83.5); // label bottom -> next ring top

// ---- ring -------------------------------------------------------------------
const RING_D: f64 = px(117.0);
const TRACK_STROKE: f64 = px(15.5);
const PROGRESS_STROKE: f64 = px(8.0);
const GLYPH: f64 = px(46.0);
const RING_LABEL_GAP: f64 = px(26.9);

// ---- card -------------------------------------------------------------------
const CARD_W: f64 = px(600.0);
const CARD_CORNER: f64 = px(49.5);
const CARD_PAD: f64 = px(32.0);
const FILLET_R: f64 = px(36.0); // smooth concave fillet joining rail into panel
const KAPPA: f64 = 0.6; // symmetric squircle control point ratio (~0.6)
const BAR_H: f64 = px(10.5);
const HEADER_GAP: f64 = px(17.0); // mark -> title
const HEADER_TO_BLOCK: f64 = px(21.0);
const LABEL_TO_BAR: f64 = px(16.8);
const BAR_TO_USED: f64 = px(17.8);
const BLOCK_SPACING: f64 = px(20.0);

// ---- menu -------------------------------------------------------------------
// Upstream has no menu (macOS gives it one for free). These follow the card's
// proportions rather than inventing a second visual language.
const MENU_W: f64 = px(400.0);
const MENU_ROW_H: f64 = px(74.0);
const MENU_PAD: f64 = px(20.0);

/// The menu's items, in order. `main` maps the index to an action.
pub const MENU: [&str; 2] = ["Refresh now", "Quit linotch"];

// ---- type -------------------------------------------------------------------
const FONT_FAMILY: &str = "SF Pro Text, Inter, Cantarell, Noto Sans, sans-serif";
const FONT_PERCENT: f64 = font_px(27.0);
const FONT_TITLE: f64 = font_px(26.0);
const FONT_BODY: f64 = font_px(18.0);

/// Apple Dynamic Island palette: pure black body, luminous semantic accents,
/// SF gray secondary text, and semi-transparent white tracks.
const NOTCH_RGB: (f64, f64, f64) = (0.0, 0.0, 0.0);
const RING_TRACK: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.11);
const BAR_TRACK: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.11);
const AMPLE: (f64, f64, f64) = (48.0 / 255.0, 209.0 / 255.0, 88.0 / 255.0); // #30D158 (iOS green)
const WATCH: (f64, f64, f64) = (1.0, 214.0 / 255.0, 10.0 / 255.0); // #FFD60A (iOS yellow)
const CRITICAL: (f64, f64, f64) = (1.0, 69.0 / 255.0, 58.0 / 255.0); // #FF453A (iOS red)
/// Anything with no colour of its own: upstream's Claude mark colour, which is
/// also the app's own accent.
pub const FALLBACK: (f64, f64, f64) = (0.851, 0.467, 0.341); // #d97757
const TEXT: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 1.0);
const TEXT_SECONDARY: (f64, f64, f64, f64) = (142.0 / 255.0, 142.0 / 255.0, 147.0 / 255.0, 1.0); // #8E8E93

/// codenotch's bands: under half is ample, then watch, then critical. A ring with
/// an accent of its own — media, coloured from the player's icon — is outside the
/// usage scale and keeps it.
pub fn band(fraction: f64, accent: Option<(f64, f64, f64)>) -> (f64, f64, f64) {
    if let Some(c) = accent {
        return c;
    }
    match fraction {
        f if f < 0.50 => AMPLE,
        f if f < 0.70 => WATCH,
        _ => CRITICAL,
    }
}

/// Panel opacity, set once from the environment. Upstream's notch is solid black;
/// this is the one knob that departs from it, and it defaults to matching.
static OPACITY: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

pub fn set_opacity(v: f64) {
    let _ = OPACITY.set(v.clamp(0.35, 1.0));
}

fn body_fill() -> (f64, f64, f64, f64) {
    let (r, g, b) = NOTCH_RGB;
    (r, g, b, *OPACITY.get().unwrap_or(&1.0))
}

/// Rough line box for a font size. Upstream measures this from the real font; a
/// fixed ratio is close enough here and keeps [`layout`] callable without a font
/// map, which the geometry tests need.
fn line(size: f64) -> f64 {
    (size * 1.32).round()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// What is open beside the rail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The hovered ring's readings.
    Card,
    /// The right-click menu, drawn as a card so it comes out of the notch in the
    /// same skin — GTK's own menu is a separate window the compositor puts in the
    /// middle of the screen, which is nowhere near where it was asked for.
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Open {
    /// The ring the panel points at.
    pub ring: usize,
    pub kind: Kind,
}

pub struct Layout {
    pub w: f64,
    pub h: f64,
    /// Ring centres, in the same order as the rings that produced them.
    pub centers: Vec<(f64, f64)>,
    /// The notch body.
    pub rail: Rect,
    /// The open card or menu.
    pub panel: Option<Rect>,
}

/// One cell: ring, gap, percent label.
fn cell_extent() -> f64 {
    RING_D + RING_LABEL_GAP + line(FONT_PERCENT)
}

/// How thick the body is. On a vertical edge the percent sits *below* its ring, so
/// it costs length; on a horizontal one the cells stand side by side and the label
/// hangs under each, so it costs depth instead. Sizing both the same is what left
/// the percentages clipped off a top-edge notch.
fn rail_depth(edge: Edge) -> f64 {
    if edge.vertical() {
        RAIL_DEPTH
    } else {
        RAIL_DEPTH + RING_LABEL_GAP + line(FONT_PERCENT)
    }
}

/// How much of the edge one cell takes up — the whole cell when stacked, just the
/// ring when laid out in a row.
fn along_extent(edge: Edge) -> f64 {
    if edge.vertical() {
        cell_extent()
    } else {
        RING_D
    }
}

fn rail_run(n: usize, edge: Edge) -> f64 {
    let n = n.max(1) as f64;
    2.0 * CURL + PAD_LEAD + n * along_extent(edge) + (n - 1.0) * CELL_SPACING + PAD_TRAIL
}

fn menu_height() -> f64 {
    2.0 * MENU_PAD + MENU.len() as f64 * MENU_ROW_H
}

fn panel_size(rings: &[Ring], open: Open) -> (f64, f64) {
    match open.kind {
        Kind::Menu => (MENU_W, menu_height()),
        Kind::Card => (
            CARD_W,
            rings
                .get(open.ring)
                .map(card_height)
                .unwrap_or(menu_height()),
        ),
    }
}

/// The widest a panel can ever get, so the window's depth is known before one
/// opens and the rail never has to move to make room.
fn widest_panel() -> f64 {
    CARD_W.max(MENU_W)
}

fn max_panel_depth(rings: &[Ring], edge: Edge) -> f64 {
    if edge.vertical() {
        widest_panel()
    } else {
        rings
            .iter()
            .map(card_height)
            .fold(0.0_f64, f64::max)
            .max(menu_height())
    }
}

fn card_height(ring: &Ring) -> f64 {
    let block = line(FONT_BODY) + LABEL_TO_BAR + BAR_H + BAR_TO_USED + line(FONT_BODY);
    let rows = ring.rows.len() as f64;
    let blocks = if ring.rows.is_empty() {
        0.0
    } else {
        rows * block + (rows - 1.0) * BLOCK_SPACING
    };
    let note = if ring.note.is_empty() {
        0.0
    } else {
        HEADER_TO_BLOCK + line(FONT_BODY)
    };
    let body = if blocks > 0.0 {
        HEADER_TO_BLOCK + blocks
    } else {
        0.0
    };
    2.0 * CARD_PAD + line(FONT_TITLE) + note + body
}

/// Room for the 24px ambient elevation shadow (spec 5.2).
pub const SHADOW_PAD: f64 = 24.0;

/// `hover` decides whether a card is drawn and where it points — but **not** how
/// long the window is.
///
/// That is the whole trick behind the rail staying still. The window is anchored
/// by its centre along the edge, so any change in its length moves the rail by
/// half of it; sizing the window to the hovered card made the rail jump every time
/// the pointer crossed a ring. The length is therefore computed from the tallest
/// card *any* ring could open, hovered or not, and only the depth changes.
pub fn layout(rings: &[Ring], edge: Edge, open: Option<Open>) -> Layout {
    let thick = rail_depth(edge);
    let run = rail_run(rings.len(), edge)
        .max(rings.iter().map(card_height).fold(0.0_f64, f64::max))
        .max(menu_height());
    let depth = thick
        + if open.is_some() {
            max_panel_depth(rings, edge) + SHADOW_PAD
        } else {
            0.0
        };

    let (w, h) = if edge.vertical() {
        (depth, run)
    } else {
        (run, depth)
    };
    let rail = match edge {
        Edge::Right => Rect {
            x: w - thick,
            y: 0.0,
            w: thick,
            h,
        },
        Edge::Left => Rect {
            x: 0.0,
            y: 0.0,
            w: thick,
            h,
        },
        Edge::Bottom => Rect {
            x: 0.0,
            y: h - thick,
            w,
            h: thick,
        },
        Edge::Top => Rect {
            x: 0.0,
            y: 0.0,
            w,
            h: thick,
        },
    };

    // Cells run along the body, which starts one curl in from each end. Across it,
    // a stacked ring is centred and a side-by-side one is pushed up to leave its
    // label room underneath.
    let start = CURL + PAD_LEAD;
    let inset = (RAIL_DEPTH - RING_D) / 2.0 + RING_D / 2.0;
    let centers = (0..rings.len().max(1))
        .map(|i| {
            let along = start + RING_D / 2.0 + i as f64 * (along_extent(edge) + CELL_SPACING);
            match edge {
                Edge::Right | Edge::Left => (rail.x + thick / 2.0, along),
                _ => (along, rail.y + inset),
            }
        })
        .collect::<Vec<_>>();

    // The panel grows out of the rail, anchored to the screen edge and centered
    // on the hovered ring.
    let panel = match open.and_then(|o| centers.get(o.ring).map(|c| (o, *c))) {
        None => None,
        Some((o, (cx, cy))) => {
            let (pw, ph) = panel_size(rings, o);
            let along = |v: f64, extent: f64, limit: f64| {
                (v - extent / 2.0).clamp(0.0, (limit - extent).max(0.0))
            };
            let rect = match edge {
                Edge::Right => Rect {
                    x: rail.x - pw,
                    y: along(cy, ph, h),
                    w: pw,
                    h: ph,
                },
                Edge::Left => Rect {
                    x: rail.x + thick,
                    y: along(cy, ph, h),
                    w: pw,
                    h: ph,
                },
                Edge::Bottom => Rect {
                    x: along(cx, pw, w),
                    y: rail.y - ph,
                    w: pw,
                    h: ph,
                },
                Edge::Top => Rect {
                    x: along(cx, pw, w),
                    y: rail.y + thick,
                    w: pw,
                    h: ph,
                },
            };
            Some(rect)
        }
    };

    Layout {
        w,
        h,
        centers,
        rail,
        panel,
    }
}

/// Which menu row the point is over, given the panel rect the menu was drawn in.
pub fn menu_hit(rect: Rect, x: f64, y: f64) -> Option<usize> {
    if !rect.contains(x, y) {
        return None;
    }
    let i = ((y - rect.y - MENU_PAD) / MENU_ROW_H).floor();
    (i >= 0.0 && (i as usize) < MENU.len()).then_some(i as usize)
}

/// Which ring contains the point, if any. The target is the whole cell, not the
/// stroke: a 3 pt arc is not something anyone can click on purpose.
pub fn hit(l: &Layout, edge: Edge, x: f64, y: f64) -> Option<usize> {
    if !l.rail.contains(x, y) {
        return None;
    }
    let reach = (along_extent(edge) + CELL_SPACING) / 2.0;
    l.centers.iter().position(|(cx, cy)| {
        let (along, across) = if edge.vertical() {
            ((y - cy).abs(), (x - cx).abs())
        } else {
            ((x - cx).abs(), (y - cy).abs())
        };
        along <= reach && across <= rail_depth(edge) / 2.0 + RING_LABEL_GAP
    })
}

// ------------------------------------------------------------------- shapes

fn rgba(cr: &cairo::Context, c: (f64, f64, f64, f64)) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

/// Draw an axis-aligned 90-degree continuous-curvature corner curve with symmetric handles (~0.6).
fn corner(cr: &cairo::Context, from: (f64, f64), to: (f64, f64), initial_horizontal: bool) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    if (x0 - x1).abs() < 1e-6 && (y0 - y1).abs() < 1e-6 {
        return;
    }
    let (cp1, cp2) = if initial_horizontal {
        ((x0 + (x1 - x0) * KAPPA, y0), (x1, y1 - (y1 - y0) * KAPPA))
    } else {
        ((x0, y0 + (y1 - y0) * KAPPA), (x1 - (x1 - x0) * KAPPA, y1))
    };
    cr.curve_to(cp1.0, cp1.1, cp2.0, cp2.1, x1, y1);
}

fn apply_canonical_transform(cr: &cairo::Context, rail: Rect, edge: Edge, thick: f64, length: f64) {
    cr.translate(rail.x, rail.y);
    match edge {
        Edge::Right => {}
        Edge::Left => cr.transform(cairo::Matrix::new(-1.0, 0.0, 0.0, 1.0, thick, 0.0)),
        Edge::Top => cr.transform(cairo::Matrix::new(0.0, -1.0, 1.0, 0.0, 0.0, thick)),
        Edge::Bottom => cr.transform(cairo::Matrix::new(0.0, 1.0, -1.0, 0.0, length, 0.0)),
    }
}

/// The notch body: a pill welded to one edge, with *inverse* rounded corners at
/// each end that flare back out to the bezel, so it reads as part of the edge
/// rather than a panel floating near it.
///
/// Written once for the right edge in a canonical space where the bezel is at
/// `maxX`, then transformed — the corner-versus-flare clamping below is the part
/// that took care, and four hand-written copies would mean three that are never
/// the one under the cursor when it breaks.
fn notch_path(cr: &cairo::Context, rail: Rect, edge: Edge) {
    let (thick, length) = if edge.vertical() {
        (rail.w, rail.h)
    } else {
        (rail.h, rail.w)
    };

    let wanted = CORNER.min(thick / 2.0).max(0.0);
    let curl = CURL.min(length / 2.0).min(thick - wanted).max(0.0);
    let corner_r = wanted.min((length - 2.0 * curl) / 2.0).max(0.0);
    let (top, bottom) = (curl, length - curl);

    let _ = cr.save();
    apply_canonical_transform(cr, rail, edge, thick, length);

    cr.new_path();
    cr.move_to(thick, 0.0);
    if curl > 0.0 {
        corner(cr, (thick, 0.0), (thick - curl, top), false);
    }
    cr.line_to(corner_r, top);
    if corner_r > 0.0 {
        corner(cr, (corner_r, top), (0.0, top + corner_r), true);
    }
    cr.line_to(0.0, bottom - corner_r);
    if corner_r > 0.0 {
        corner(cr, (0.0, bottom - corner_r), (corner_r, bottom), false);
    }
    cr.line_to(thick - curl, bottom);
    if curl > 0.0 {
        corner(cr, (thick - curl, bottom), (thick, length), true);
    }
    cr.close_path();
    let _ = cr.restore();
}

struct CanonicalPanel {
    v1: f64,
    v2: f64,
    depth: f64,
}

fn canonical_panel(rect: Rect, rail: Rect, edge: Edge) -> CanonicalPanel {
    match edge {
        Edge::Top => CanonicalPanel {
            v1: rect.x,
            v2: rect.x + rect.w,
            depth: rect.h,
        },
        Edge::Left | Edge::Right => CanonicalPanel {
            v1: rect.y,
            v2: rect.y + rect.h,
            depth: rect.w,
        },
        Edge::Bottom => CanonicalPanel {
            v1: rail.w - (rect.x + rect.w),
            v2: rail.w - rect.x,
            depth: rect.h,
        },
    }
}

/// Geometry parameters for the unified Dynamic Island shape. Shared by `island_path` and `draw_rim_stroke`.
struct IslandGeom {
    thick: f64,
    length: f64,
    curl: f64,
    top: f64,
    corner: f64,
    bottom: f64,
    d: f64,
    v1: f64,
    v2: f64,
    rc: f64,
    rf_left: f64,
    rf_right: f64,
}

impl IslandGeom {
    fn new(rail: Rect, edge: Edge, panel: Rect, t: f64, offset: f64, expand: f64) -> Self {
        let (thick, length) = if edge.vertical() {
            (rail.w, rail.h)
        } else {
            (rail.h, rail.w)
        };

        let wanted = CORNER.min(thick / 2.0).max(0.0);
        let curl = CURL.min(length / 2.0).min(thick - wanted).max(0.0);
        let corner_r = wanted.min((length - 2.0 * curl) / 2.0).max(0.0);
        let (top, bottom) = (curl, length - curl);

        let canon = canonical_panel(panel, rail, edge);
        let t_geom = t.clamp(0.0, 1.05);
        let base_d = (canon.depth * t_geom).max(0.0);

        // Cap expand so shadow never clips against the window boundary:
        // 1. In depth: window allocated depth is canon.depth + SHADOW_PAD.
        let max_expand_d = (canon.depth + SHADOW_PAD - base_d - offset).max(0.0);
        let expand_d = expand.min(max_expand_d).max(0.0);
        let d = (base_d + offset + expand_d).max(0.0);

        let v_mid = (canon.v1 + canon.v2) / 2.0;
        let target_hw = (canon.v2 - canon.v1) / 2.0;
        let init_hw = (RING_D / 2.0).min(target_hw);
        let hw_base = init_hw + (target_hw - init_hw) * t.clamp(0.0, 1.0);

        let min_v = top + corner_r;
        let max_v = bottom - corner_r;

        // 2. Along the edge: cap expand to room remaining before rail ends.
        let avail_left_room = (v_mid - hw_base - min_v).max(0.0);
        let avail_right_room = (max_v - (v_mid + hw_base)).max(0.0);
        let expand_v = expand.min(avail_left_room.min(avail_right_room)).max(0.0);

        let hw = (hw_base + expand_v).max(0.0);
        let v1 = (v_mid - hw).clamp(min_v, max_v);
        let v2 = (v_mid + hw).clamp(v1, max_v);

        let rc = (CARD_CORNER + expand_d)
            .min(d / 2.0)
            .min((v2 - v1) / 2.0)
            .max(0.0);

        let avail_left = v1 - min_v;
        let rf_left = FILLET_R
            .min(d / 2.0)
            .min((v2 - v1) / 4.0)
            .min(avail_left)
            .max(0.0);

        let avail_right = max_v - v2;
        let rf_right = FILLET_R
            .min(d / 2.0)
            .min((v2 - v1) / 4.0)
            .min(avail_right)
            .max(0.0);

        Self {
            thick,
            length,
            curl,
            top,
            corner: corner_r,
            bottom,
            d,
            v1,
            v2,
            rc,
            rf_left,
            rf_right,
        }
    }

    /// Traces the open panel perimeter from the start of the left fillet to the end of the right fillet.
    fn draw_panel_perimeter(&self, cr: &cairo::Context) {
        if self.rf_left > 0.0 {
            corner(
                cr,
                (0.0, self.v1 - self.rf_left),
                (-self.rf_left, self.v1),
                false,
            );
        }
        cr.line_to(-self.d + self.rc, self.v1);
        if self.rc > 0.0 {
            corner(
                cr,
                (-self.d + self.rc, self.v1),
                (-self.d, self.v1 + self.rc),
                true,
            );
        }
        cr.line_to(-self.d, self.v2 - self.rc);
        if self.rc > 0.0 {
            corner(
                cr,
                (-self.d, self.v2 - self.rc),
                (-self.d + self.rc, self.v2),
                false,
            );
        }
        cr.line_to(-self.rf_right, self.v2);
        if self.rf_right > 0.0 {
            corner(
                cr,
                (-self.rf_right, self.v2),
                (0.0, self.v2 + self.rf_right),
                true,
            );
        }
    }
}

/// The unified Dynamic Island body: rail + panel as one continuous topological shape,
/// with smooth concave fillets joining the rail into the panel and continuous-curvature
/// squircles on all corners. Morph smoothly with parameter `t`. Supports outward `offset`
/// and concentric `expand` for the ambient elevation shadow.
fn island_path(
    cr: &cairo::Context,
    rail: Rect,
    edge: Edge,
    panel: Rect,
    t: f64,
    offset: f64,
    expand: f64,
) {
    let geom = IslandGeom::new(rail, edge, panel, t, offset, expand);
    if geom.d < 0.5 {
        notch_path(cr, rail, edge);
        return;
    }

    let _ = cr.save();
    apply_canonical_transform(cr, rail, edge, geom.thick, geom.length);

    cr.new_path();
    cr.move_to(geom.thick, 0.0);
    if geom.curl > 0.0 {
        corner(
            cr,
            (geom.thick, 0.0),
            (geom.thick - geom.curl, geom.top),
            false,
        );
    }
    cr.line_to(geom.corner, geom.top);
    if geom.corner > 0.0 {
        corner(
            cr,
            (geom.corner, geom.top),
            (0.0, geom.top + geom.corner),
            true,
        );
    }
    cr.line_to(0.0, geom.v1 - geom.rf_left);
    geom.draw_panel_perimeter(cr);
    cr.line_to(0.0, geom.bottom - geom.corner);
    if geom.corner > 0.0 {
        corner(
            cr,
            (0.0, geom.bottom - geom.corner),
            (geom.corner, geom.bottom),
            false,
        );
    }
    cr.line_to(geom.thick - geom.curl, geom.bottom);
    if geom.curl > 0.0 {
        corner(
            cr,
            (geom.thick - geom.curl, geom.bottom),
            (geom.thick, geom.length),
            true,
        );
    }
    cr.close_path();
    let _ = cr.restore();
}

/// 3-tier concentric ambient gradient shadow (Apple elevation shadow, spec 5.2):
/// 1. Atmospheric feather: offset 6px downward, expand 18px, alpha 0.08
/// 2. Ambient bloom: offset 3px downward, expand 8px, alpha 0.18
/// 3. Contact shadow: offset 1px downward, expand 2px, alpha 0.35
fn draw_ambient_shadow(cr: &cairo::Context, rail: Rect, edge: Edge, panel: Rect, t: f64) {
    if t <= 0.05 {
        return;
    }
    let shadow_alpha = ((t - 0.1) / 0.9).clamp(0.0, 1.0);
    const PASSES: [(f64, f64, f64); 3] = [(6.0, 18.0, 0.08), (3.0, 8.0, 0.18), (1.0, 2.0, 0.35)];
    for (offset, expand, alpha) in PASSES {
        island_path(cr, rail, edge, panel, t, offset, expand);
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha * shadow_alpha);
        let _ = cr.fill();
    }
}

/// 1px luminous rim highlight along the panel perimeter, rendered with a subtle
/// vertical gradient (top ~0.22 white alpha -> bottom ~0.04).
fn draw_rim_stroke(cr: &cairo::Context, rail: Rect, edge: Edge, panel: Rect, t: f64) {
    if t <= 0.05 {
        return;
    }
    let geom = IslandGeom::new(rail, edge, panel, t, 0.0, 0.0);
    if geom.d < 1.0 {
        return;
    }
    let rim_alpha = ((t - 0.1) / 0.9).clamp(0.0, 1.0);

    let _ = cr.save();
    apply_canonical_transform(cr, rail, edge, geom.thick, geom.length);

    cr.new_path();
    cr.move_to(0.0, geom.v1 - geom.rf_left);
    geom.draw_panel_perimeter(cr);

    let g = cairo::LinearGradient::new(0.0, 0.0, -geom.d, 0.0);
    g.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.22 * rim_alpha);
    g.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.04 * rim_alpha);
    cr.set_line_width(1.0);
    if let Ok(()) = cr.set_source(&g) {
        let _ = cr.stroke();
    }
    let _ = cr.restore();
}

fn squircle_path(cr: &cairo::Context, rect: Rect, r: f64) {
    let Rect { x, y, w, h } = rect;
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_path();
    if r <= 0.0 {
        cr.rectangle(x, y, w, h);
        return;
    }
    cr.move_to(x + r, y);
    cr.line_to(x + w - r, y);
    corner(cr, (x + w - r, y), (x + w, y + r), true);
    cr.line_to(x + w, y + h - r);
    corner(cr, (x + w, y + h - r), (x + w - r, y + h), false);
    cr.line_to(x + r, y + h);
    corner(cr, (x + r, y + h), (x, y + h - r), true);
    cr.line_to(x, y + r);
    corner(cr, (x, y + r), (x + r, y), false);
    cr.close_path();
}

fn bar(cr: &cairo::Context, x: f64, y: f64, w: f64, frac: f64, color: (f64, f64, f64), alpha: f64) {
    squircle_path(cr, Rect { x, y, w, h: BAR_H }, BAR_H / 2.0);
    rgba(cr, BAR_TRACK);
    let _ = cr.fill();
    let fw = (w * frac.clamp(0.0, 1.0)).max(BAR_H);
    squircle_path(
        cr,
        Rect {
            x,
            y,
            w: fw,
            h: BAR_H,
        },
        BAR_H / 2.0,
    );
    cr.set_source_rgba(color.0, color.1, color.2, alpha);
    let _ = cr.fill();
}

// --------------------------------------------------------------------- text

/// Pango text layout with support for tabular figures (`tnum`) on numbers.
#[allow(clippy::too_many_arguments)]
fn text(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    s: &str,
    size: f64,
    weight: pango::Weight,
    color: (f64, f64, f64, f64),
    align: pango::Alignment,
    tnum: bool,
) {
    let layout = pangocairo::functions::create_layout(cr);
    let mut fd = pango::FontDescription::new();
    fd.set_family(FONT_FAMILY);
    fd.set_weight(weight);
    fd.set_absolute_size(size * pango::SCALE as f64);
    layout.set_font_description(Some(&fd));
    layout.set_text(s);
    layout.set_width((w * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_alignment(align);
    if tnum {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrFontFeatures::new("tnum=1"));
        layout.set_attributes(Some(&attrs));
    }
    cr.move_to(x, y);
    rgba(cr, color);
    pangocairo::functions::show_layout(cr, &layout);
}

// --------------------------------------------------------------------- draw

/// `t` is the panel's open-ness: 0 shut, 1 open, and a little past 1 while the
/// spring overshoots. Content fades in over t 0.35->1.0 and is not drawn below 0.2.
pub fn draw(
    cr: &cairo::Context,
    rings: &[Ring],
    l: &Layout,
    edge: Edge,
    open: Option<Open>,
    t: f64,
    hot: Option<usize>,
) {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    if let (Some(rect), Some(o)) = (l.panel, open)
        && t > 0.004
    {
        draw_ambient_shadow(cr, l.rail, edge, rect, t);
        island_path(cr, l.rail, edge, rect, t, 0.0, 0.0);
        rgba(cr, body_fill());
        let _ = cr.fill();

        draw_rim_stroke(cr, l.rail, edge, rect, t);

        if t >= 0.2 {
            let content_alpha = ((t - 0.35) / 0.65).clamp(0.0, 1.0);
            if content_alpha > 0.001 {
                let _ = cr.save();
                island_path(cr, l.rail, edge, rect, t, 0.0, 0.0);
                cr.clip();
                let draw_content = |cr: &cairo::Context| match o.kind {
                    Kind::Card => {
                        if let Some(ring) = rings.get(o.ring) {
                            card(cr, rect, ring);
                        }
                    }
                    Kind::Menu => menu(cr, rect, hot),
                };
                if content_alpha < 1.0 {
                    cr.push_group();
                    draw_content(cr);
                    let _ = cr.pop_group_to_source();
                    let _ = cr.paint_with_alpha(content_alpha);
                } else {
                    draw_content(cr);
                }
                let _ = cr.restore();
            }
        }
    } else {
        notch_path(cr, l.rail, edge);
        rgba(cr, body_fill());
        let _ = cr.fill();
    }

    for (i, (ring, &(cx, cy))) in rings.iter().zip(l.centers.iter()).enumerate() {
        let lit = match open {
            Some(o) if o.ring == i => t.clamp(0.0, 1.0),
            _ => 0.0,
        };
        ring_at(cr, ring, cx, cy, lit);
    }
}

fn menu(cr: &cairo::Context, rect: Rect, hot: Option<usize>) {
    for (i, item) in MENU.iter().enumerate() {
        let y = rect.y + MENU_PAD + i as f64 * MENU_ROW_H;
        if hot == Some(i) {
            squircle_path(
                cr,
                Rect {
                    x: rect.x + MENU_PAD * 0.4,
                    y,
                    w: rect.w - MENU_PAD * 0.8,
                    h: MENU_ROW_H,
                },
                MENU_ROW_H * 0.32,
            );
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.11);
            let _ = cr.fill();
        }
        text(
            cr,
            rect.x + MENU_PAD,
            y + (MENU_ROW_H - line(FONT_BODY)) / 2.0,
            rect.w - 2.0 * MENU_PAD,
            item,
            FONT_BODY,
            pango::Weight::Normal,
            TEXT,
            pango::Alignment::Left,
            false,
        );
    }
}

/// A media ring takes its colour from the player's own icon; everything else
/// grades on usage.
fn accent_of(ring: &Ring) -> Option<(f64, f64, f64)> {
    match &ring.glyph {
        Glyph::Player { desktop_entry, .. } => {
            Some(icons::dominant(desktop_entry, 32).unwrap_or(FALLBACK))
        }
        _ => None,
    }
}

fn ring_at(cr: &cairo::Context, ring: &Ring, cx: f64, cy: f64, lit: f64) {
    let a = ring.alpha();
    // strokeBorder: the track sits inside the diameter, so both strokes share the
    // same centre radius and the thin arc rides down the middle of the thick one.
    let r = (RING_D - TRACK_STROKE) / 2.0;

    if lit > 0.004 {
        // A halo rather than a size change: growing the ring would slide the mark
        // out from under the pointer that is hovering it.
        cr.new_path();
        cr.set_line_width(1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * lit);
        cr.arc(cx, cy, RING_D / 2.0 + px(14.0) * lit, 0.0, 2.0 * PI);
        let _ = cr.stroke();
    }

    cr.new_path();
    cr.set_line_width(TRACK_STROKE);
    cr.set_line_cap(cairo::LineCap::Butt);
    cr.set_source_rgba(RING_TRACK.0, RING_TRACK.1, RING_TRACK.2, RING_TRACK.3 * a);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let _ = cr.stroke();

    if ring.fraction > 0.0 {
        let (cr_, cg, cb) = band(ring.fraction, accent_of(ring));
        cr.new_path();
        cr.set_line_width(PROGRESS_STROKE);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_source_rgba(cr_, cg, cb, a);
        let start = -PI / 2.0;
        cr.arc(
            cx,
            cy,
            r,
            start,
            start + 2.0 * PI * ring.fraction.clamp(0.0, 1.0),
        );
        let _ = cr.stroke();
    }

    cr.new_path();
    match &ring.glyph {
        Glyph::Brand { asset, color } => {
            icons::brand(cr, asset, cx, cy, GLYPH, (color.0, color.1, color.2, a));
        }
        Glyph::Player {
            desktop_entry,
            playing,
        } => {
            if !icons::app(cr, desktop_entry, cx, cy, GLYPH as i32) {
                transport(cr, cx, cy, *playing, a);
            }
        }
    }

    if !ring.percent.is_empty() {
        let w = RING_D * 2.0;
        text(
            cr,
            cx - w / 2.0,
            cy + RING_D / 2.0 + RING_LABEL_GAP,
            w,
            &ring.percent,
            FONT_PERCENT,
            pango::Weight::Semibold,
            (TEXT.0, TEXT.1, TEXT.2, TEXT.3 * a),
            pango::Alignment::Center,
            true,
        );
    }
}

fn transport(cr: &cairo::Context, cx: f64, cy: f64, playing: bool, alpha: f64) {
    cr.new_path();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    let s = GLYPH / 2.0;
    if playing {
        let (w, h) = (s * 0.34, s * 1.25);
        cr.rectangle(cx - s * 0.58, cy - h / 2.0, w, h);
        cr.rectangle(cx + s * 0.24, cy - h / 2.0, w, h);
    } else {
        // Nudged right: a triangle's visual centre sits left of its bounding box.
        cr.move_to(cx - s * 0.55, cy - s * 0.72);
        cr.line_to(cx + s * 0.78, cy);
        cr.line_to(cx - s * 0.55, cy + s * 0.72);
        cr.close_path();
    }
    let _ = cr.fill();
}

fn card(cr: &cairo::Context, rect: Rect, ring: &Ring) {
    let x = rect.x + CARD_PAD;
    let w = rect.w - CARD_PAD * 2.0;
    let mut y = rect.y + CARD_PAD;

    match &ring.glyph {
        Glyph::Brand { asset, color } => {
            icons::brand(
                cr,
                asset,
                x + GLYPH / 2.0,
                y + line(FONT_TITLE) / 2.0,
                GLYPH,
                (color.0, color.1, color.2, 1.0),
            );
        }
        Glyph::Player {
            desktop_entry,
            playing,
        } => {
            let (gx, gy) = (x + GLYPH / 2.0, y + line(FONT_TITLE) / 2.0);
            if !icons::app(cr, desktop_entry, gx, gy, GLYPH as i32) {
                transport(cr, gx, gy, *playing, 1.0);
            }
        }
    }
    let tx = x + GLYPH + HEADER_GAP;
    text(
        cr,
        tx,
        y,
        rect.w - CARD_PAD - tx + rect.x,
        &ring.label,
        FONT_TITLE,
        pango::Weight::Semibold,
        TEXT,
        pango::Alignment::Left,
        false,
    );
    y += line(FONT_TITLE);

    if !ring.note.is_empty() {
        y += HEADER_TO_BLOCK;
        text(
            cr,
            x,
            y,
            w,
            &ring.note,
            FONT_BODY,
            pango::Weight::Normal,
            TEXT_SECONDARY,
            pango::Alignment::Left,
            false,
        );
        y += line(FONT_BODY);
    }

    for (i, row) in ring.rows.iter().enumerate() {
        y += if i == 0 {
            HEADER_TO_BLOCK
        } else {
            BLOCK_SPACING
        };
        // label left, reset right, on one line
        text(
            cr,
            x,
            y,
            w * 0.62,
            &row.label,
            FONT_BODY,
            pango::Weight::Normal,
            TEXT,
            pango::Alignment::Left,
            false,
        );
        if !row.note.is_empty() {
            text(
                cr,
                x,
                y,
                w,
                &row.note,
                FONT_BODY,
                pango::Weight::Normal,
                TEXT_SECONDARY,
                pango::Alignment::Right,
                true,
            );
        }
        y += line(FONT_BODY);
        if let Some(f) = row.bar {
            y += LABEL_TO_BAR;
            bar(cr, x, y, w, f, band(f, accent_of(ring)), ring.alpha());
            y += BAR_H + BAR_TO_USED;
        }
        text(
            cr,
            x,
            y,
            w,
            &row.value,
            FONT_BODY,
            pango::Weight::Normal,
            TEXT,
            pango::Alignment::Left,
            true,
        );
        y += line(FONT_BODY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::{Health, Row};

    fn ring(rows: usize) -> Ring {
        let mut r = Ring::usage("Test", "claude", (1.0, 1.0, 1.0));
        r.health = Health::Ok;
        r.percent = "42%".into();
        r.rows = (0..rows)
            .map(|i| Row {
                label: format!("w{i}"),
                value: "50% Used".into(),
                bar: Some(0.5),
                note: "Resets in 1h".into(),
            })
            .collect();
        r
    }

    fn card_on(ring: usize) -> Option<Open> {
        Some(Open {
            ring,
            kind: Kind::Card,
        })
    }

    #[test]
    fn every_drawn_ring_is_clickable_at_its_own_centre() {
        let rings: Vec<Ring> = (0..4).map(|_| ring(2)).collect();
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            for hover in [None, card_on(0), card_on(3)] {
                let l = layout(&rings, edge, hover);
                for (i, &(x, y)) in l.centers.iter().enumerate() {
                    assert_eq!(
                        hit(&l, edge, x, y),
                        Some(i),
                        "{edge:?} hover={hover:?} ring {i}"
                    );
                    assert!(l.rail.contains(x, y), "{edge:?} ring {i} not on the body");
                }
            }
        }
    }

    /// The regression that mattered: the window is centred on its edge, so any
    /// change in its length along that edge moves the rail by half of it. Opening
    /// a card must not change that length — only the depth.
    #[test]
    fn opening_a_card_never_changes_the_length_along_the_edge() {
        for count in 1..=4 {
            let rings: Vec<Ring> = (0..count).map(|i| ring(i % 3 + 1)).collect();
            for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
                let shut = layout(&rings, edge, None);
                for hover in 0..rings.len() {
                    let open = layout(&rings, edge, card_on(hover));
                    let (a, b) = if edge.vertical() {
                        (shut.h, open.h)
                    } else {
                        (shut.w, open.w)
                    };
                    assert_eq!(a, b, "{edge:?} n={count} hover={hover}: length changed");
                    // Only the along-axis is checked: the window is anchored to its
                    // edge, so growth in depth pushes the far side away and leaves
                    // the rail where it was on screen.
                    let along = |c: &(f64, f64)| if edge.vertical() { c.1 } else { c.0 };
                    let before: Vec<f64> = shut.centers.iter().map(along).collect();
                    let after: Vec<f64> = open.centers.iter().map(along).collect();
                    assert_eq!(
                        before, after,
                        "{edge:?} n={count} hover={hover}: rings moved along the edge"
                    );
                }
            }
        }
    }

    #[test]
    fn the_card_stays_inside_the_window() {
        let rings: Vec<Ring> = (0..3).map(|_| ring(3)).collect();
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            for hover in 0..rings.len() {
                let l = layout(&rings, edge, card_on(hover));
                let c = l.panel.expect("hover opens a card");
                assert!(c.x >= 0.0 && c.y >= 0.0, "{edge:?} card off the top/left");
                assert!(
                    c.x + c.w <= l.w + 0.01 && c.y + c.h <= l.h + 0.01,
                    "{edge:?} card overflows"
                );
            }
        }
    }

    #[test]
    fn menu_rows_are_hittable_and_the_gaps_are_not() {
        let rings: Vec<Ring> = (0..3).map(|_| ring(1)).collect();
        let l = layout(
            &rings,
            Edge::Right,
            Some(Open {
                ring: 1,
                kind: Kind::Menu,
            }),
        );
        let r = l.panel.expect("the menu is a panel");
        for i in 0..MENU.len() {
            let y = r.y + MENU_PAD + (i as f64 + 0.5) * MENU_ROW_H;
            assert_eq!(menu_hit(r, r.x + r.w / 2.0, y), Some(i), "row {i}");
        }
        assert_eq!(menu_hit(r, r.x + r.w / 2.0, r.y + 1.0), None, "top padding");
        assert_eq!(menu_hit(r, r.x - 5.0, r.y + r.h / 2.0), None, "outside");
    }

    /// Bands are upstream's, and the boundaries are the part worth pinning.
    #[test]
    fn bands_match_upstream() {
        assert_eq!(band(0.0, None), AMPLE);
        assert_eq!(band(0.499, None), AMPLE);
        assert_eq!(band(0.5, None), WATCH);
        assert_eq!(band(0.699, None), WATCH);
        assert_eq!(band(0.7, None), CRITICAL);
        assert_eq!(band(1.5, None), CRITICAL);
        // An accent overrides the scale outright.
        assert_eq!(band(0.9, Some(FALLBACK)), FALLBACK);
    }

    #[test]
    fn island_path_has_no_diagonal_rail_shortcuts_when_panel_clamped() {
        let rings: Vec<Ring> = (0..3).map(|_| ring(3)).collect();
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            for hover in [0, rings.len() - 1] {
                let l = layout(&rings, edge, card_on(hover));
                let panel = l.panel.expect("hover opens a card");
                let surf = cairo::ImageSurface::create(
                    cairo::Format::ARgb32,
                    l.w as i32 + 1,
                    l.h as i32 + 1,
                )
                .unwrap();
                let cr = cairo::Context::new(&surf).unwrap();
                island_path(&cr, l.rail, edge, panel, 1.0, 0.0, 0.0);
                let path = cr.copy_path().unwrap();
                let mut prev = (0.0, 0.0);
                for seg in path.iter() {
                    match seg {
                        cairo::PathSegment::MoveTo((x, y)) => prev = (x, y),
                        cairo::PathSegment::LineTo((x, y)) => {
                            let dx = (x - prev.0).abs();
                            let dy = (y - prev.1).abs();
                            assert!(
                                dx < 1e-4 || dy < 1e-4,
                                "{edge:?} hover {hover}: diagonal line from ({:.1}, {:.1}) to ({:.1}, {:.1}) (dx={:.1}, dy={:.1})",
                                prev.0,
                                prev.1,
                                x,
                                y,
                                dx,
                                dy
                            );
                            prev = (x, y);
                        }
                        cairo::PathSegment::CurveTo(_, _, (x, y)) => prev = (x, y),
                        cairo::PathSegment::ClosePath => {}
                    }
                }
            }
        }
    }

    #[test]
    fn centered_panel_and_notch_flares_are_mirror_symmetric() {
        let rings: Vec<Ring> = (0..3).map(|_| ring(3)).collect();
        let l = layout(&rings, Edge::Right, None);
        let mid = l.rail.h / 2.0;
        let card_h = 40.0;
        let centered_panel = Rect {
            x: l.rail.x - 50.0,
            y: mid - card_h / 2.0,
            w: 50.0,
            h: card_h,
        };
        let geom = IslandGeom::new(l.rail, Edge::Right, centered_panel, 1.0, 0.0, 0.0);
        assert!(
            (geom.v1 - (mid - card_h / 2.0)).abs() < 1e-4,
            "v1 should be centered around mid"
        );
        assert!(
            (geom.v2 - (mid + card_h / 2.0)).abs() < 1e-4,
            "v2 should be centered around mid"
        );
        assert!(
            (geom.rf_left - geom.rf_right).abs() < 1e-4,
            "fillet radii are not mirror symmetric: left={:.2}, right={:.2}",
            geom.rf_left,
            geom.rf_right
        );
        assert!(
            (geom.top - (geom.length - geom.bottom)).abs() < 1e-4,
            "flares are not mirror symmetric: top={:.2}, bottom_dist={:.2}",
            geom.top,
            geom.length - geom.bottom
        );
    }
}

/// Renders the notch to a PNG with the same readings as upstream's design frame,
/// so the two can be put side by side. Writes a file and needs a font map, so it
/// is not part of the default run: `cargo test -- --ignored preview`.
#[cfg(test)]
#[test]
#[ignore]
fn preview() {
    use crate::ring::{Action, Health, Row};

    fn provider(name: &str, asset: &'static str, pct: f64, rows: Vec<Row>) -> Ring {
        let mut r = Ring::usage(&format!("{name} Usage"), asset, (1.0, 1.0, 1.0));
        r.health = Health::Ok;
        r.fraction = pct;
        r.percent = format!("{:.0}%", pct * 100.0);
        r.rows = rows;
        r
    }
    fn row(label: &str, pct: f64, resets: &str) -> Row {
        Row {
            label: label.into(),
            value: format!("{:.0}% Used", pct * 100.0),
            bar: Some(pct),
            note: format!("Resets {resets}"),
        }
    }

    let rings = vec![
        provider(
            "Claude",
            "claude",
            0.73,
            vec![
                row("Current session", 0.73, "in 51 min"),
                row("All models", 0.07, "Thu 12:00 AM"),
            ],
        ),
        provider(
            "Codex",
            "openai",
            0.21,
            vec![row("Current session", 0.21, "in 3h 20m")],
        ),
        Ring {
            label: "Spotify".into(),
            percent: "2:14".into(),
            note: "Bohemian Rhapsody".into(),
            rows: vec![Row {
                label: "Queen".into(),
                value: "2:14 / 5:03".into(),
                bar: Some(0.44),
                note: "Playing".into(),
            }],
            fraction: 0.44,
            glyph: Glyph::Player {
                desktop_entry: "spotify".into(),
                playing: true,
            },
            health: Health::Ok,
            action: Some(Action::PlayPause),
        },
    ];

    let pad = 26.0;
    let gap = 26.0;

    let edges = [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom];
    struct RowInfo {
        edge: Edge,
        items: Vec<(Layout, Option<Open>, f64, Option<usize>)>,
        w: f64,
        h: f64,
    }

    let rows: Vec<RowInfo> = edges
        .into_iter()
        .map(|edge| {
            let shut = layout(&rings, edge, None);
            let card_half = layout(
                &rings,
                edge,
                Some(Open {
                    ring: 0,
                    kind: Kind::Card,
                }),
            );
            let card_full = layout(
                &rings,
                edge,
                Some(Open {
                    ring: 0,
                    kind: Kind::Card,
                }),
            );
            let menu = layout(
                &rings,
                edge,
                Some(Open {
                    ring: 2,
                    kind: Kind::Menu,
                }),
            );

            let items = vec![
                (shut, None, 0.0, None),
                (
                    card_half,
                    Some(Open {
                        ring: 0,
                        kind: Kind::Card,
                    }),
                    0.5,
                    None,
                ),
                (
                    card_full,
                    Some(Open {
                        ring: 0,
                        kind: Kind::Card,
                    }),
                    1.0,
                    None,
                ),
                (
                    menu,
                    Some(Open {
                        ring: 2,
                        kind: Kind::Menu,
                    }),
                    1.0,
                    Some(0),
                ),
            ];

            let w = pad * 2.0
                + items.iter().map(|(l, ..)| l.w).sum::<f64>()
                + gap * (items.len() - 1) as f64;
            let h = items.iter().map(|(l, ..)| l.h).fold(0.0_f64, f64::max);

            RowInfo { edge, items, w, h }
        })
        .collect();

    let total_w = rows.iter().map(|r| r.w).fold(0.0_f64, f64::max) as i32;
    let total_h =
        (pad * 2.0 + rows.iter().map(|r| r.h).sum::<f64>() + gap * (rows.len() - 1) as f64) as i32;

    let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, total_w, total_h).unwrap();
    let cr = cairo::Context::new(&surf).unwrap();

    let mut y_cursor = pad;
    for row in &rows {
        let mut x_cursor = pad;
        for (l, open, t, hot) in &row.items {
            cr.save().unwrap();
            cr.translate(x_cursor, y_cursor);
            cr.rectangle(0.0, 0.0, l.w, l.h);
            cr.clip();
            draw(&cr, &rings, l, row.edge, *open, *t, *hot);
            cr.restore().unwrap();
            x_cursor += l.w + gap;
        }
        y_cursor += row.h + gap;
    }

    // Backdrop gradient underneath
    cr.set_operator(cairo::Operator::DestOver);
    let g = cairo::LinearGradient::new(0.0, 0.0, total_w as f64, total_h as f64);
    g.add_color_stop_rgb(0.0, 0.16, 0.55, 0.62);
    g.add_color_stop_rgb(0.5, 0.42, 0.32, 0.22);
    g.add_color_stop_rgb(1.0, 0.10, 0.38, 0.45);
    cr.set_source(&g).unwrap();
    cr.paint().unwrap();
    drop(cr);

    let mut f = std::fs::File::create("/tmp/linotch-preview.png").unwrap();
    surf.write_to_png(&mut f).unwrap();
    eprintln!("wrote /tmp/linotch-preview.png ({total_w}x{total_h})");
}
