//! Cairo drawing, and the geometry that click handling shares with it.
//!
//! [`layout`] is computed once per frame and used for painting, hit testing and the
//! window's input region, so a ring can never be drawn in one place, clicked in
//! another, and swallow the pointer in a third.
//!
//! The rail — the pill with the rings — is centred in the window along the free
//! axis. That matters: the window grows when a card opens, and centring is what
//! keeps the rings from sliding across the screen as it does.

use crate::icons;
use crate::ring::{Glyph, Ring};
use crate::surface::Edge;
use gtk::pango;
use std::f64::consts::PI;

const RAIL: f64 = 56.0; // thickness of the pill
const RING_D: f64 = 36.0;
const STROKE: f64 = 3.5;
const GAP: f64 = 14.0;
const PAD: f64 = 10.0;
const PILL_R: f64 = 18.0;

const CARD_W: f64 = 284.0;
const CARD_GAP: f64 = 10.0;
const CARD_R: f64 = 14.0;
const CARD_PAD: f64 = 14.0;
const ROW_H: f64 = 36.0;
const HEAD_H: f64 = 30.0;

// One palette, so a colour is never invented halfway down the file.
const BG: (f64, f64, f64, f64) = (0.043, 0.043, 0.051, 0.94);
const HAIRLINE: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.10);
const TRACK: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.09);
const INK: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.95);
const INK_DIM: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.44);
const DIVIDER: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.07);

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

pub struct Layout {
    pub w: f64,
    pub h: f64,
    /// Ring centres, in the same order as the rings that produced them.
    pub centers: Vec<(f64, f64)>,
    pub r: f64,
    /// The pill.
    pub rail: Rect,
    /// The open card, when one ring is hovered.
    pub card: Option<Rect>,
}

fn card_height(ring: &Ring) -> f64 {
    let note = if ring.note.is_empty() { 0.0 } else { 17.0 };
    CARD_PAD * 2.0 + HEAD_H + note + ring.rows.len() as f64 * ROW_H
}

/// `hover` is the ring the pointer is over, which is what opens a card.
pub fn layout(rings: &[Ring], edge: Edge, hover: Option<usize>) -> Layout {
    let n = rings.len().max(1) as f64;
    let rail_run = n * RING_D + (n - 1.0) * GAP + 2.0 * PAD;
    let card_h = hover.and_then(|i| rings.get(i)).map(card_height);
    let card_run = card_h.unwrap_or(0.0);
    let grow = if card_h.is_some() { CARD_W + CARD_GAP } else { 0.0 };

    let (w, h, rail, card_origin) = if edge.vertical() {
        // Rail hugs left or right; the card opens inward beside it.
        let h = rail_run.max(card_run);
        let w = RAIL + grow;
        let rail_x = if edge == Edge::Right { w - RAIL } else { 0.0 };
        let card_x = if edge == Edge::Right { 0.0 } else { RAIL + CARD_GAP };
        (
            w,
            h,
            Rect { x: rail_x, y: (h - rail_run) / 2.0, w: RAIL, h: rail_run },
            (card_x, 0.0),
        )
    } else {
        let w = rail_run.max(CARD_W);
        let h = RAIL + if card_h.is_some() { CARD_GAP + card_run } else { 0.0 };
        let rail_y = if edge == Edge::Bottom { h - RAIL } else { 0.0 };
        let card_y = if edge == Edge::Bottom { 0.0 } else { RAIL + CARD_GAP };
        (
            w,
            h,
            Rect { x: (w - rail_run) / 2.0, y: rail_y, w: rail_run, h: RAIL },
            (0.0, card_y),
        )
    };

    let centers = (0..rings.len().max(1))
        .map(|i| {
            let along = PAD + RING_D / 2.0 + i as f64 * (RING_D + GAP);
            if edge.vertical() {
                (rail.x + RAIL / 2.0, rail.y + along)
            } else {
                (rail.x + along, rail.y + RAIL / 2.0)
            }
        })
        .collect::<Vec<_>>();

    // The card lines up with the ring that opened it, then is pulled back inside
    // the window — a card hanging off the top edge would simply be clipped away.
    let card = card_h.map(|ch| {
        let (cx, cy) = centers[hover.unwrap_or(0).min(centers.len() - 1)];
        if edge.vertical() {
            Rect { x: card_origin.0, y: (cy - ch / 2.0).clamp(0.0, (h - ch).max(0.0)), w: CARD_W, h: ch }
        } else {
            Rect { x: (cx - CARD_W / 2.0).clamp(0.0, (w - CARD_W).max(0.0)), y: card_origin.1, w: CARD_W, h: ch }
        }
    });

    Layout { w, h, centers, r: (RING_D - STROKE) / 2.0, rail, card }
}

/// Which ring contains the point, if any. The target is the whole cell, not the
/// stroke: a 3.5 px ring is not something anyone can click on purpose.
pub fn hit(l: &Layout, x: f64, y: f64) -> Option<usize> {
    l.centers.iter().position(|(cx, cy)| {
        let (dx, dy) = (x - cx, y - cy);
        (dx * dx + dy * dy).sqrt() <= RING_D / 2.0 + GAP / 2.0
    })
}

// ------------------------------------------------------------------- shapes

fn rounded(cr: &cairo::Context, rect: Rect, corners: (f64, f64, f64, f64)) {
    let Rect { x, y, w, h } = rect;
    let (tl, tr, br, bl) = corners;
    cr.new_path();
    cr.new_sub_path();
    cr.arc(x + w - tr, y + tr, tr, -PI / 2.0, 0.0);
    cr.arc(x + w - br, y + h - br, br, 0.0, PI / 2.0);
    cr.arc(x + bl, y + h - bl, bl, PI / 2.0, PI);
    cr.arc(x + tl, y + tl, tl, PI, 1.5 * PI);
    cr.close_path();
}

fn rgba(cr: &cairo::Context, c: (f64, f64, f64, f64)) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

/// Panel with a hairline edge — the hairline is most of what separates "a dark
/// rectangle" from "a surface".
fn panel(cr: &cairo::Context, rect: Rect, corners: (f64, f64, f64, f64)) {
    rounded(cr, rect, corners);
    rgba(cr, BG);
    let _ = cr.fill_preserve();
    // A short fall of light from the top edge. Flat fill plus hairline reads as a
    // rectangle; this reads as a surface with a direction to it.
    let g = cairo::LinearGradient::new(rect.x, rect.y, rect.x, rect.y + rect.h.min(90.0));
    g.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.055);
    g.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    let _ = cr.set_source(&g);
    let _ = cr.fill_preserve();
    rgba(cr, HAIRLINE);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

/// Rounded progress bar. `frac` of the track is filled in `color`.
fn bar(cr: &cairo::Context, x: f64, y: f64, w: f64, frac: f64, color: (f64, f64, f64), alpha: f64) {
    let h = 5.0;
    rounded(cr, Rect { x, y, w, h }, (2.5, 2.5, 2.5, 2.5));
    rgba(cr, TRACK);
    let _ = cr.fill();
    let fw = (w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h } else { 0.0 });
    if fw > 0.0 {
        rounded(cr, Rect { x, y, w: fw, h }, (2.5, 2.5, 2.5, 2.5));
        cr.set_source_rgba(color.0, color.1, color.2, alpha);
        let _ = cr.fill();
    }
}

// --------------------------------------------------------------------- text

/// Pango rather than cairo's toy text API: it kerns, it handles non-ASCII track
/// titles, and it ellipsizes — all three show up the moment a real song plays.
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
) {
    let layout = pangocairo::functions::create_layout(cr);
    let mut fd = pango::FontDescription::new();
    fd.set_family("Sans");
    fd.set_weight(weight);
    fd.set_absolute_size(size * pango::SCALE as f64);
    layout.set_font_description(Some(&fd));
    layout.set_text(s);
    layout.set_width((w * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_alignment(align);
    cr.move_to(x, y);
    rgba(cr, color);
    pangocairo::functions::show_layout(cr, &layout);
}

// --------------------------------------------------------------------- draw

pub fn draw(cr: &cairo::Context, rings: &[Ring], l: &Layout, edge: Edge, hover: Option<usize>) {
    // The window is an ARGB surface; clear it or the previous frame shows through.
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    if let (Some(rect), Some(ring)) = (l.card, hover.and_then(|i| rings.get(i))) {
        card(cr, rect, ring);
    }

    // Pill: the screen-facing edge stays square, so the notch reads as growing out
    // of the bezel rather than floating near it.
    let sq = 0.0;
    let corners = match edge {
        Edge::Right => (PILL_R, sq, sq, PILL_R),
        Edge::Left => (sq, PILL_R, PILL_R, sq),
        Edge::Top => (sq, sq, PILL_R, PILL_R),
        Edge::Bottom => (PILL_R, PILL_R, sq, sq),
    };
    panel(cr, l.rail, corners);

    for (i, (ring, &(cx, cy))) in rings.iter().zip(l.centers.iter()).enumerate() {
        ring_at(cr, ring, cx, cy, l.r, hover == Some(i));
    }
}

fn ring_at(cr: &cairo::Context, ring: &Ring, cx: f64, cy: f64, r: f64, hot: bool) {
    let a = ring.alpha();
    let (cr_, cg, cb) = ring.color();
    cr.set_line_cap(cairo::LineCap::Round);

    if hot {
        // A halo instead of a size change: growing the ring would move the mark
        // under the pointer and make the whole rail feel unstable.
        cr.new_path();
        cr.set_line_width(1.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.13);
        cr.arc(cx, cy, r + 6.0, 0.0, 2.0 * PI);
        let _ = cr.stroke();
    }

    cr.new_path();
    cr.set_line_width(STROKE);
    rgba(cr, TRACK);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let _ = cr.stroke();

    if ring.fraction > 0.0 {
        let start = -PI / 2.0;
        let end = start + 2.0 * PI * ring.fraction.clamp(0.0, 1.0);
        // Soft bloom under the arc — the one thing that stops the ring reading as
        // a flat stroke on a flat panel.
        cr.new_path();
        cr.set_line_width(STROKE + 5.0);
        cr.set_source_rgba(cr_, cg, cb, 0.16 * a);
        cr.arc(cx, cy, r, start, end);
        let _ = cr.stroke();

        cr.new_path();
        cr.set_line_width(STROKE);
        cr.set_source_rgba(cr_, cg, cb, a);
        cr.arc(cx, cy, r, start, end);
        let _ = cr.stroke();
    }

    cr.new_path();
    match &ring.glyph {
        Glyph::Brand { asset, color } => {
            icons::brand(cr, asset, cx, cy, 17.0, (color.0, color.1, color.2, a));
        }
        Glyph::Player { desktop_entry, playing } => {
            // At rest: whose player this is. Under the pointer: what a click does.
            if hot || !icons::app(cr, desktop_entry, cx, cy, 20) {
                transport(cr, cx, cy, *playing, a);
            }
        }
    }
}

fn transport(cr: &cairo::Context, cx: f64, cy: f64, playing: bool, alpha: f64) {
    cr.new_path();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92 * alpha);
    if playing {
        let (w, h) = (3.0, 11.0);
        cr.rectangle(cx - 5.0, cy - h / 2.0, w, h);
        cr.rectangle(cx + 2.0, cy - h / 2.0, w, h);
    } else {
        // Nudged right: a triangle's visual centre sits left of its bounding box.
        let s = 5.5;
        cr.move_to(cx - s * 0.6, cy - s);
        cr.line_to(cx + s, cy);
        cr.line_to(cx - s * 0.6, cy + s);
        cr.close_path();
    }
    let _ = cr.fill();
}

fn card(cr: &cairo::Context, rect: Rect, ring: &Ring) {
    panel(cr, rect, (CARD_R, CARD_R, CARD_R, CARD_R));

    let x = rect.x + CARD_PAD;
    let w = rect.w - CARD_PAD * 2.0;
    let mut y = rect.y + CARD_PAD;

    // Heading: mark, then name, then the headline number on the right.
    match &ring.glyph {
        Glyph::Brand { asset, color } => {
            icons::brand(cr, asset, x + 8.0, y + 8.0, 16.0, (color.0, color.1, color.2, 1.0));
        }
        Glyph::Player { desktop_entry, playing } => {
            if !icons::app(cr, desktop_entry, x + 8.0, y + 8.0, 16) {
                transport(cr, x + 8.0, y + 8.0, *playing, 1.0);
            }
        }
    }
    text(cr, x + 24.0, y - 2.0, w - 24.0, &ring.label, 13.0, pango::Weight::Bold, INK, pango::Alignment::Left);
    y += HEAD_H - 8.0;

    // Hairline under the heading: it is what makes the rows read as a list rather
    // than as text that happens to be below a title.
    cr.new_path();
    cr.set_line_width(1.0);
    rgba(cr, DIVIDER);
    cr.move_to(x, y.round() + 0.5);
    cr.line_to(x + w, y.round() + 0.5);
    let _ = cr.stroke();
    y += 8.0;

    if !ring.note.is_empty() {
        text(cr, x, y - 2.0, w, &ring.note, 11.0, pango::Weight::Normal, INK_DIM, pango::Alignment::Left);
        y += 17.0;
    }

    for row in &ring.rows {
        text(cr, x, y + 1.0, w * 0.6, &row.label, 11.5, pango::Weight::Normal, INK_DIM, pango::Alignment::Left);
        text(cr, x, y, w, &row.value, 13.0, pango::Weight::Bold, INK, pango::Alignment::Right);
        if let Some(f) = row.bar {
            let bw = if row.note.is_empty() { w } else { w * 0.62 };
            bar(cr, x, y + 22.0, bw, f, ring.color_for(f), ring.alpha());
            if !row.note.is_empty() {
                text(
                    cr,
                    x + bw + 8.0,
                    y + 17.0,
                    w - bw - 8.0,
                    &row.note,
                    10.5,
                    pango::Weight::Normal,
                    INK_DIM,
                    pango::Alignment::Right,
                );
            }
        }
        y += ROW_H;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::{Health, Row};

    fn ring(rows: usize) -> Ring {
        let mut r = Ring::usage("Test", "claude", (1.0, 1.0, 1.0));
        r.health = Health::Ok;
        r.rows = (0..rows)
            .map(|i| Row {
                label: format!("w{i}"),
                value: "50%".into(),
                bar: Some(0.5),
                note: "resets in 1h".into(),
            })
            .collect();
        r
    }

    #[test]
    fn every_drawn_ring_is_clickable_at_its_own_centre() {
        let rings: Vec<Ring> = (0..4).map(|_| ring(2)).collect();
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            for hover in [None, Some(0), Some(3)] {
                let l = layout(&rings, edge, hover);
                for (i, &(x, y)) in l.centers.iter().enumerate() {
                    assert_eq!(hit(&l, x, y), Some(i), "{edge:?} hover={hover:?} ring {i}");
                    assert!(l.rail.contains(x, y), "{edge:?} ring {i} not on the pill");
                }
            }
        }
    }

    /// The rail must not move when a card opens, or the rings slide out from under
    /// the pointer that opened them.
    #[test]
    fn opening_a_card_leaves_the_rings_where_they_were() {
        let rings: Vec<Ring> = (0..3).map(|_| ring(3)).collect();
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            let shut = layout(&rings, edge, None);
            let open = layout(&rings, edge, Some(1));
            for (i, (&(ax, ay), &(bx, by))) in
                shut.centers.iter().zip(open.centers.iter()).enumerate()
            {
                // Positions are window-relative; the window itself is re-anchored by
                // the same deltas, so what must match is the offset from the rail.
                let (dax, day) = (ax - shut.rail.x, ay - shut.rail.y);
                let (dbx, dby) = (bx - open.rail.x, by - open.rail.y);
                assert_eq!((dax, day), (dbx, dby), "{edge:?} ring {i} moved in the rail");
            }
        }
    }

    #[test]
    fn the_card_stays_inside_the_window() {
        let rings: Vec<Ring> = (0..2).map(|_| ring(3)).collect();
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            for hover in 0..rings.len() {
                let l = layout(&rings, edge, Some(hover));
                let c = l.card.expect("hover opens a card");
                assert!(c.x >= 0.0 && c.y >= 0.0, "{edge:?} card off the top/left");
                assert!(c.x + c.w <= l.w + 0.01 && c.y + c.h <= l.h + 0.01, "{edge:?} card overflows");
                assert!(
                    !l.rail.contains(-1.0, -1.0) && !c.contains(-1.0, -1.0),
                    "{edge:?} outside must stay click-through"
                );
            }
        }
    }
}

/// Renders the notch to a PNG with representative data, both shut and with a card
/// open. Not part of the default run — it writes a file and needs a font map:
/// `cargo test -- --ignored preview`, then look at `/tmp/linotch-preview.png`.
#[cfg(test)]
#[test]
#[ignore]
fn preview() {
    use crate::ring::{Action, Health, Row};

    fn row(label: &str, pct: f64, note: &str) -> Row {
        Row {
            label: label.into(),
            value: format!("{:.0}%", pct * 100.0),
            bar: Some(pct),
            note: note.into(),
        }
    }
    let mut claude = Ring::usage("Claude", "claude", (0.851, 0.467, 0.341));
    claude.health = Health::Ok;
    claude.fraction = 0.42;
    claude.rows = vec![
        row("Session", 0.42, "in 2h 14m"),
        row("Weekly (all)", 0.18, "in 3d 4h"),
    ];
    let mut codex = Ring::usage("Codex", "openai", (1.0, 1.0, 1.0));
    codex.health = Health::Ok;
    codex.fraction = 0.71;
    codex.rows = vec![row("Session", 0.71, "in 48m"), row("Weekly", 0.33, "in 5d")];
    let media = Ring {
        label: "Spotify".into(),
        note: "Bohemian Rhapsody".into(),
        rows: vec![Row {
            label: "Queen".into(),
            value: "2:14".into(),
            bar: Some(0.37),
            note: "5:03".into(),
        }],
        fraction: 0.37,
        glyph: Glyph::Player { desktop_entry: "spotify".into(), playing: true },
        health: Health::Ok,
        neutral: true,
        action: Some(Action::PlayPause),
    };
    let rings = vec![claude, codex, media];

    let shut = layout(&rings, Edge::Right, None);
    let open = layout(&rings, Edge::Right, Some(0));
    let (pad, gap) = (24.0, 24.0);
    let w = (pad * 2.0 + shut.w + gap + open.w) as i32;
    let h = (pad * 2.0 + shut.h.max(open.h)) as i32;

    let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
    let cr = cairo::Context::new(&surf).unwrap();

    for (l, hover, x) in [
        (&shut, None, pad),
        (&open, Some(0), pad + shut.w + gap),
    ] {
        cr.save().unwrap();
        cr.translate(x, pad);
        cr.rectangle(0.0, 0.0, l.w, l.h);
        cr.clip();
        draw(&cr, &rings, l, Edge::Right, hover);
        cr.restore().unwrap();
    }
    // draw() clears its own clip first, so the backdrop goes on underneath at the
    // end rather than being painted first and wiped.
    cr.set_operator(cairo::Operator::DestOver);
    cr.set_source_rgb(0.13, 0.14, 0.16);
    cr.paint().unwrap();
    drop(cr);

    let mut f = std::fs::File::create("/tmp/linotch-preview.png").unwrap();
    surf.write_to_png(&mut f).unwrap();
    eprintln!("wrote /tmp/linotch-preview.png ({w}x{h})");
}
