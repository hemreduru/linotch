//! Cairo drawing, and the geometry that click handling shares with it.
//!
//! [`Layout`] is computed once per frame and used for both painting and hit
//! testing, so a ring can never be drawn in one place and clicked in another.

use crate::ring::{Glyph, Ring};
use crate::surface::Edge;
use std::f64::consts::PI;

const RING_D: f64 = 34.0;
const STROKE: f64 = 4.0;
const GAP: f64 = 10.0;
const PAD: f64 = 10.0;
const RADIUS: f64 = 16.0;

pub struct Layout {
    pub w: f64,
    pub h: f64,
    /// Ring centres, in the same order as the rings that produced them.
    pub centers: Vec<(f64, f64)>,
    pub r: f64,
}

pub fn layout(n: usize, edge: Edge) -> Layout {
    let n = n.max(1) as f64;
    let run = n * RING_D + (n - 1.0) * GAP + 2.0 * PAD;
    let thick = RING_D + 2.0 * PAD;
    let (w, h) = if edge.vertical() {
        (thick, run)
    } else {
        (run, thick)
    };
    let centers = (0..n as usize)
        .map(|i| {
            let along = PAD + RING_D / 2.0 + i as f64 * (RING_D + GAP);
            if edge.vertical() {
                (w / 2.0, along)
            } else {
                (along, h / 2.0)
            }
        })
        .collect();
    Layout {
        w,
        h,
        centers,
        r: (RING_D - STROKE) / 2.0,
    }
}

/// Which ring contains the point, if any. The target is the full cell, not the
/// stroke: a 4 px ring is not a click target anyone can hit on purpose.
pub fn hit(l: &Layout, x: f64, y: f64) -> Option<usize> {
    l.centers.iter().position(|(cx, cy)| {
        let (dx, dy) = (x - cx, y - cy);
        (dx * dx + dy * dy).sqrt() <= RING_D / 2.0 + GAP / 2.0
    })
}

/// Rounded rectangle with the screen-facing edge left square, so the notch reads as
/// growing out of the bezel rather than floating near it.
fn pill(cr: &cairo::Context, l: &Layout, edge: Edge) {
    let (w, h, r) = (l.w, l.h, RADIUS);
    let (tl, tr, br, bl) = match edge {
        Edge::Right => (r, 0.0, 0.0, r),
        Edge::Left => (0.0, r, r, 0.0),
        Edge::Top => (0.0, 0.0, r, r),
        Edge::Bottom => (r, r, 0.0, 0.0),
    };
    cr.new_sub_path();
    cr.arc(w - tr, tr, tr, -PI / 2.0, 0.0);
    cr.arc(w - br, h - br, br, 0.0, PI / 2.0);
    cr.arc(bl, h - bl, bl, PI / 2.0, PI);
    cr.arc(tl, tl, tl, PI, 1.5 * PI);
    cr.close_path();
}

pub fn draw(cr: &cairo::Context, rings: &[Ring], l: &Layout, edge: Edge) {
    // The window is an ARGB surface; clear it or the previous frame shows through.
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    cr.new_path();
    pill(cr, l, edge);
    cr.set_source_rgba(0.04, 0.04, 0.05, 0.88);
    let _ = cr.fill();

    for (ring, &(cx, cy)) in rings.iter().zip(l.centers.iter()) {
        let a = ring.alpha();
        cr.set_line_width(STROKE);
        cr.set_line_cap(cairo::LineCap::Round);

        // Track: always drawn, so a ring with no reading is still visibly a ring
        // rather than a gap in the pill.
        // new_path() before every arc: cairo joins an arc to whatever point the path
        // was left at, and the glyph's text leaves one — without this, a line is
        // drawn from each ring's glyph to the next ring's arc.
        cr.new_path();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        cr.arc(cx, cy, l.r, 0.0, 2.0 * PI);
        let _ = cr.stroke();

        if ring.fraction > 0.0 {
            let (r, g, b) = ring.color();
            cr.set_source_rgba(r, g, b, a);
            let start = -PI / 2.0;
            cr.new_path();
            cr.arc(cx, cy, l.r, start, start + 2.0 * PI * ring.fraction.clamp(0.0, 1.0));
            let _ = cr.stroke();
        }

        glyph(cr, &ring.glyph, cx, cy, a);
    }
}

fn glyph(cr: &cairo::Context, g: &Glyph, cx: f64, cy: f64, alpha: f64) {
    cr.new_path();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.86 * alpha);
    match g {
        Glyph::Text(t) => {
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(13.0);
            if let Ok(e) = cr.text_extents(t) {
                cr.move_to(
                    cx - e.width() / 2.0 - e.x_bearing(),
                    cy - e.height() / 2.0 - e.y_bearing(),
                );
                let _ = cr.show_text(t);
            }
            // show_text leaves a current point behind; the next arc would join to it.
            cr.new_path();
        }
        Glyph::Play => {
            // Nudged right by a third of its width: a triangle's visual centre is
            // left of its bounding box, and centred by the box it looks off.
            let s = 5.5;
            cr.move_to(cx - s * 0.6, cy - s);
            cr.line_to(cx + s, cy);
            cr.line_to(cx - s * 0.6, cy + s);
            cr.close_path();
            let _ = cr.fill();
        }
        Glyph::Pause => {
            let (w, h) = (3.0, 11.0);
            cr.rectangle(cx - 5.0, cy - h / 2.0, w, h);
            cr.rectangle(cx + 2.0, cy - h / 2.0, w, h);
            let _ = cr.fill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drawn_ring_is_clickable_at_its_own_centre() {
        for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
            let l = layout(4, edge);
            for (i, &(x, y)) in l.centers.iter().enumerate() {
                assert_eq!(hit(&l, x, y), Some(i), "{edge:?} ring {i}");
                assert!(x >= 0.0 && x <= l.w && y >= 0.0 && y <= l.h, "{edge:?} ring {i} outside");
            }
        }
    }

    #[test]
    fn a_click_in_the_corner_hits_nothing() {
        let l = layout(3, Edge::Right);
        assert_eq!(hit(&l, 0.5, 0.5), None);
    }
}
