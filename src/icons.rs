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
pub fn brand(cr: &cairo::Context, name: &str, cx: f64, cy: f64, size: f64, rgba: (f64, f64, f64, f64)) {
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
