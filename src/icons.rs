//! Marks drawn inside the rings.
//!
//! Two kinds, loaded differently on purpose:
//!
//! * **Brand marks** are single-colour PNGs generated from the upstream SVGs and
//!   embedded in the binary. They are used as an *alpha mask*, so one asset can be
//!   painted in any colour — brand colour normally, dimmed when a reading is stale.
//! * **Application icons** come from the icon theme, by the `DesktopEntry` the media
//!   player publishes. Those are full colour and drawn as-is, because a recoloured
//!   Spotify or Firefox icon stops being recognisable.
//!
//! Both are cached: this runs on every frame.

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// White-on-transparent renders of the lobe-icons SVGs (MIT). White because a mask
/// only cares about alpha — the colour comes from the caller.
const EMBEDDED: &[(&str, &[u8])] = &[
    ("claude", include_bytes!("../assets/claude.png")),
    ("openai", include_bytes!("../assets/openai.png")),
];

thread_local! {
    static MASKS: RefCell<HashMap<&'static str, Option<cairo::ImageSurface>>> =
        RefCell::new(HashMap::new());
    static APPS: RefCell<HashMap<String, Option<Pixbuf>>> = RefCell::new(HashMap::new());
}

/// Paint an embedded brand mark centred on (cx, cy), `size` px wide, in `rgba`.
pub fn brand(
    cr: &cairo::Context,
    name: &str,
    cx: f64,
    cy: f64,
    size: f64,
    rgba: (f64, f64, f64, f64),
) {
    MASKS.with(|c| {
        let mut c = c.borrow_mut();
        let key = match EMBEDDED.iter().find(|(n, _)| *n == name) {
            Some((n, _)) => *n,
            None => return,
        };
        let surface = c.entry(key).or_insert_with(|| {
            let bytes = EMBEDDED.iter().find(|(n, _)| *n == key).map(|(_, b)| *b)?;
            cairo::ImageSurface::create_from_png(&mut &bytes[..]).ok()
        });
        let Some(s) = surface else { return };

        let scale = size / s.width() as f64;
        let _ = cr.save();
        cr.translate(cx - size / 2.0, cy - size / 2.0);
        cr.scale(scale, scale);
        let (r, g, b, a) = rgba;
        cr.set_source_rgba(r, g, b, a);
        // mask_surface uses only the surface's alpha, so the mark takes our colour.
        let _ = cr.mask_surface(s, 0.0, 0.0);
        let _ = cr.restore();
    });
}

/// Paint a themed application icon centred on (cx, cy). Falls back to nothing when
/// the theme has no such icon, and the caller draws its own glyph instead.
pub fn app(cr: &cairo::Context, desktop_entry: &str, cx: f64, cy: f64, size: i32) -> bool {
    APPS.with(|c| {
        let mut c = c.borrow_mut();
        let pixbuf = c
            .entry(format!("{desktop_entry}@{size}"))
            .or_insert_with(|| lookup(desktop_entry, size));
        let Some(p) = pixbuf else { return false };
        let _ = cr.save();
        cr.set_source_pixbuf(p, cx - p.width() as f64 / 2.0, cy - p.height() as f64 / 2.0);
        let _ = cr.paint();
        let _ = cr.restore();
        true
    })
}

/// An icon's average colour, or `None` when it has no colour worth using.
type Accent = Option<(f64, f64, f64)>;

thread_local! {
    static DOMINANT: RefCell<HashMap<String, Accent>> = RefCell::new(HashMap::new());
}

/// The colour an application icon reads as, for the ring drawn around it.
///
/// Not a plain average — that lands on grey for anything with a light background,
/// and Chrome's icon is mostly white. Pixels are weighted by how saturated they
/// are, so the ring picks up what the eye does: Spotify green, Firefox orange,
/// VLC's traffic cone.
pub fn dominant(desktop_entry: &str, size: i32) -> Accent {
    DOMINANT.with(|c| {
        *c.borrow_mut()
            .entry(desktop_entry.to_string())
            .or_insert_with(|| {
                let p = lookup(desktop_entry, size)?;
                let (w, h, stride, chans) = (p.width(), p.height(), p.rowstride(), p.n_channels());
                if chans < 3 {
                    return None;
                }
                let bytes = unsafe { p.pixels() };
                let (mut sr, mut sg, mut sb, mut weight) = (0.0, 0.0, 0.0, 0.0);
                for y in 0..h {
                    for x in 0..w {
                        let i = (y * stride + x * chans) as usize;
                        if i + chans as usize > bytes.len() {
                            continue;
                        }
                        let (r, g, b) = (
                            bytes[i] as f64 / 255.0,
                            bytes[i + 1] as f64 / 255.0,
                            bytes[i + 2] as f64 / 255.0,
                        );
                        let alpha = if chans == 4 {
                            bytes[i + 3] as f64 / 255.0
                        } else {
                            1.0
                        };
                        let max = r.max(g).max(b);
                        let min = r.min(g).min(b);
                        // Saturation x alpha: transparent padding and grey chrome
                        // both contribute nothing.
                        let sat = if max <= 0.0 { 0.0 } else { (max - min) / max };
                        let wgt = sat * sat * alpha;
                        sr += r * wgt;
                        sg += g * wgt;
                        sb += b * wgt;
                        weight += wgt;
                    }
                }
                // A monochrome icon has nothing to say; the caller falls back.
                if weight < 1.0 {
                    return None;
                }
                let (r, g, b) = (sr / weight, sg / weight, sb / weight);
                // Lift it clear of the black body, keeping the hue.
                let max = r.max(g).max(b);
                let lift = if max < 0.55 {
                    0.55 / max.max(0.01)
                } else {
                    1.0
                };
                Some((
                    (r * lift).min(1.0),
                    (g * lift).min(1.0),
                    (b * lift).min(1.0),
                ))
            })
    })
}

/// MPRIS publishes a desktop-entry name; icon themes key on that, on a lowercased
/// form, and — for Chromium and friends — on a `-browser` suffixed variant.
fn lookup(desktop_entry: &str, size: i32) -> Option<Pixbuf> {
    // IconTheme::default() panics rather than failing when GTK is not up, which the
    // offscreen preview render and any headless test would otherwise hit.
    if !gtk::is_initialized() {
        return None;
    }
    let theme = gtk::IconTheme::default()?;
    let lower = desktop_entry.to_lowercase();
    let candidates = [
        desktop_entry.to_string(),
        lower.clone(),
        lower.replace('.', "-"),
        // "org.mozilla.firefox" → "firefox"
        lower.rsplit('.').next().unwrap_or(&lower).to_string(),
    ];
    for name in candidates {
        if let Ok(Some(p)) = theme.load_icon(&name, size, gtk::IconLookupFlags::FORCE_SIZE) {
            return Some(p);
        }
    }
    None
}
