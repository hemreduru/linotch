//! linotch — a usage notch for Linux.
//!
//! A pill on a screen edge: one ring per coding assistant showing how much of its
//! limit is gone, plus one for whatever is playing, which doubles as a play/pause
//! button. Hovering a ring opens a card beside it.
//!
//! Threading: one worker owns every read (HTTP and D-Bus both block) and publishes
//! into a mutex; GTK only ever paints what it finds there. Nothing blocking runs on
//! the UI thread, which is why the notch keeps redrawing while a provider hangs.

mod draw;
mod icons;
mod media;
mod providers;
mod ring;
mod surface;

use gtk::prelude::*;
use gtk::{gdk, glib};
use providers::Provider;
use ring::{Action, Glyph, Health, Ring, Row};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use surface::Edge;

// Three minutes, not one: a limit window does not move fast enough to be worth a
// per-minute poll, and the usage endpoint answers a burst with a Retry-After
// measured in half hours.
const POLL_OK: Duration = Duration::from_secs(180);
const POLL_NO_CREDENTIAL: Duration = Duration::from_secs(300);
const BACKOFF_BASE: Duration = Duration::from_secs(60);
const BACKOFF_CAP: Duration = Duration::from_secs(900);

/// Set by the tray menu, cleared by the worker: an immediate re-read without
/// waiting out POLL_OK.
static REFRESH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Default)]
struct Shared {
    rings: Vec<Ring>,
    /// Bus name of the player the media ring was drawn for, so a click acts on the
    /// player that was on screen and not on whatever started since.
    media_bus: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--check") => return check(),
        Some("--version") => return println!("linotch {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h") => return help(),
        Some(other) => {
            eprintln!("linotch: unknown argument {other:?}");
            help();
            std::process::exit(2);
        }
        None => {}
    }

    let edge = std::env::var("LINOTCH_EDGE")
        .ok()
        .and_then(|s| Edge::parse(&s))
        .unwrap_or(Edge::Right);
    let offset = std::env::var("LINOTCH_OFFSET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.5);
    if let Some(o) = std::env::var("LINOTCH_OPACITY").ok().and_then(|s| s.parse().ok()) {
        draw::set_opacity(o);
    }

    let shared = Arc::new(Mutex::new(Shared::default()));
    std::thread::spawn({
        let shared = Arc::clone(&shared);
        move || worker(shared)
    });

    let app = gtk::Application::builder()
        .application_id("dev.linotch.Linotch")
        .build();
    app.connect_activate(move |app| build_ui(app, Arc::clone(&shared), edge, offset));
    // GTK must not see our own flags; they were handled above.
    app.run_with_args::<&str>(&[]);
}

fn help() {
    println!(
        "linotch {} — usage notch for Linux

USAGE:
    linotch              run the notch
    linotch --check      print what each source reports, then exit
    linotch --version

ENVIRONMENT:
    LINOTCH_EDGE=right|left|top|bottom   screen edge to hug        (default: right)
    LINOTCH_OFFSET=0.0..1.0              position along that edge  (default: 0.5)
    LINOTCH_OPACITY=0.35..1.0            panel opacity             (default: 1.0)
    LINOTCH_SURFACE=layer|x11|floating   override display-server detection",
        env!("CARGO_PKG_VERSION")
    );
}

/// Terminal self-check: the first thing to run when a ring is missing.
fn check() {
    println!("linotch {}\n", env!("CARGO_PKG_VERSION"));
    println!(
        "session: {}",
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "?".into())
    );
    println!(
        "desktop: {}\n",
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into())
    );

    for p in providers::all() {
        if !p.present() {
            println!("{:8} not installed", p.label());
            continue;
        }
        match p.read() {
            Ok(ws) if ws.is_empty() => println!("{:8} no limit windows reported", p.label()),
            Ok(ws) => {
                println!("{:8} ok", p.label());
                for w in ws {
                    let when = w
                        .resets_at
                        .map(providers::until)
                        .unwrap_or_else(|| "?".into());
                    println!("         {:16} {:5.1}%  resets {}", w.label, w.used * 100.0, when);
                }
            }
            Err(e) => println!("{:8} {e}", p.label()),
        }
    }

    match zbus::blocking::Connection::session().map(|c| media::poll(&c)) {
        Ok(Some(m)) => println!(
            "{:8} {} [{}] — {} ({})",
            "media",
            m.identity,
            m.desktop_entry,
            media::describe(&m),
            if m.playing { "playing" } else { "paused" }
        ),
        Ok(None) => println!("{:8} nothing playing", "media"),
        Err(e) => println!("{:8} no session bus: {e}", "media"),
    }
}

// ------------------------------------------------------------------ worker

struct Slot {
    provider: Box<dyn Provider>,
    ring: Ring,
    /// Consecutive failures, for the backoff exponent.
    fails: u32,
    next: Instant,
}

fn worker(shared: Arc<Mutex<Shared>>) {
    let mut slots: Vec<Slot> = providers::all()
        .into_iter()
        .filter(|p| p.present())
        .map(|p| Slot {
            ring: Ring::usage(&format!("{} Usage", p.label()), p.asset(), p.brand()),
            provider: p,
            fails: 0,
            next: Instant::now(),
        })
        .collect();

    // One connection for the life of the process: the session bus is cheap to hold
    // and wasteful to reopen every second.
    let bus = zbus::blocking::Connection::session().ok();

    loop {
        let now = Instant::now();
        let forced = REFRESH.swap(false, std::sync::atomic::Ordering::Relaxed);
        for s in &mut slots {
            if now < s.next && !forced {
                continue;
            }
            match s.provider.read() {
                Ok(ws) if !ws.is_empty() => {
                    // The ring shows the window closest to its limit — the one that
                    // will actually stop you — while the card lists them all.
                    let worst = ws
                        .iter()
                        .max_by(|a, b| a.used.total_cmp(&b.used))
                        .expect("non-empty");
                    s.ring.fraction = worst.used;
                    s.ring.percent = format!("{:.0}%", worst.used * 100.0);
                    s.ring.health = Health::Ok;
                    s.ring.note.clear();
                    s.ring.rows = ws
                        .iter()
                        .map(|w| Row {
                            label: w.label.clone(),
                            value: format!("{:.0}% Used", w.used * 100.0),
                            bar: Some(w.used),
                            note: w
                                .resets_at
                                .map(|t| format!("Resets {}", providers::until(t)))
                                .unwrap_or_default(),
                        })
                        .collect();
                    s.fails = 0;
                    s.next = now + POLL_OK;
                }
                Ok(_) => {
                    s.ring.health = Health::Idle;
                    s.ring.note = "no limit windows reported".into();
                    s.next = now + POLL_OK;
                }
                Err(
                    e @ (providers::Error::NoCredential
                    | providers::Error::Expired
                    | providers::Error::Rejected { .. }),
                ) => {
                    s.ring.health = Health::NeedsAuth;
                    s.ring.fraction = 0.0;
                    s.ring.percent.clear();
                    s.ring.rows.clear();
                    s.ring.note = e.to_string();
                    s.next = now + POLL_NO_CREDENTIAL;
                }
                Err(e) => {
                    // Never invent a number: the last reading stays, dimmed, and the
                    // card says why it is old.
                    s.fails = s.fails.saturating_add(1);
                    if s.ring.health == Health::Ok {
                        s.ring.health = Health::Stale;
                    }
                    // The vendor's own Retry-After wins whenever it is longer than
                    // our backoff — asking again before it expires is what keeps a
                    // rate limit alive instead of letting it lapse.
                    let wait = match e {
                        providers::Error::RateLimited { retry_after: Some(s) } => {
                            Duration::from_secs(s).max(backoff(1))
                        }
                        _ => backoff(s.fails),
                    };
                    s.ring.note = if s.ring.rows.is_empty() {
                        e.to_string()
                    } else {
                        format!("{e} — showing the last reading")
                    };
                    s.next = now + wait;
                }
            }
        }

        let m = bus.as_ref().and_then(media::poll);
        let mut rings: Vec<Ring> = slots.iter().map(|s| s.ring.clone()).collect();
        let media_bus = m.as_ref().map(|m| m.bus.clone());
        if let Some(m) = m {
            rings.push(media_ring(&m));
        }

        if let Ok(mut g) = shared.lock() {
            g.rings = rings;
            g.media_bus = media_bus;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn media_ring(m: &media::Media) -> Ring {
    Ring {
        label: m.identity.clone(),
        percent: if m.has_progress { clock(m.position) } else { String::new() },
        note: m.title.clone(),
        rows: vec![Row {
            label: if m.artist.is_empty() {
                if m.playing { "Playing".into() } else { "Paused".into() }
            } else {
                m.artist.clone()
            },
            value: if m.has_progress {
                format!("{} / {}", clock(m.position), clock(m.length))
            } else {
                "Live".into()
            },
            bar: Some(if m.has_progress { m.progress } else { 0.0 }),
            note: if m.playing { "Playing".into() } else { "Paused".into() },
        }],
        // No length published (a stream, most browser tabs) means no honest progress
        // to draw — the bare track says "playing, length unknown".
        fraction: if m.has_progress { m.progress } else { 0.0 },
        glyph: Glyph::Player {
            desktop_entry: m.desktop_entry.clone(),
            playing: m.playing,
        },
        // Paused is a current fact, not an old reading: dimming it would say "this
        // number may be wrong", which is not what is meant.
        health: Health::Ok,
        neutral: true,
        action: Some(Action::PlayPause),
    }
}

/// Seconds as `m:ss`, or `h:mm:ss` past an hour.
fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn backoff(fails: u32) -> Duration {
    BACKOFF_BASE
        .saturating_mul(1u32 << fails.min(4))
        .min(BACKOFF_CAP)
}

// ---------------------------------------------------------------------- UI

fn build_ui(app: &gtk::Application, shared: Arc<Mutex<Shared>>, edge: Edge, offset: f64) {
    let surface: Rc<dyn surface::Surface> = surface::detect().into();
    eprintln!("linotch: surface = {}", surface.name());

    let win = gtk::ApplicationWindow::new(app);
    win.set_app_paintable(true);
    win.set_resizable(false);
    // Without an ARGB visual the panel's corners come out black instead of clear.
    if let Some(v) = gtk::prelude::WidgetExt::screen(&win).and_then(|s| s.rgba_visual()) {
        win.set_visual(Some(&v));
    }
    surface.prepare(&win, edge, offset);

    let area = gtk::DrawingArea::new();
    area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK,
    );
    win.add(&area);

    let hover: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let shown = Rc::new(Cell::new((0usize, None::<usize>)));

    area.connect_draw({
        let shared = Arc::clone(&shared);
        let hover = Rc::clone(&hover);
        move |_, cr| {
            let rings = shared.lock().map(|g| g.rings.clone()).unwrap_or_default();
            let h = hover.get().filter(|i| *i < rings.len());
            let l = draw::layout(&rings, edge, h);
            draw::draw(cr, &rings, &l, edge, h);
            glib::Propagation::Stop
        }
    });

    area.connect_motion_notify_event({
        let shared = Arc::clone(&shared);
        let hover = Rc::clone(&hover);
        let win = win.clone();
        let area_ = area.clone();
        let surface = Rc::clone(&surface);
        let shown = Rc::clone(&shown);
        move |_, ev| {
            let rings = shared.lock().map(|g| g.rings.clone()).unwrap_or_default();
            let current = hover.get();
            let l = draw::layout(&rings, edge, current);
            let (x, y) = ev.position();
            let next = match draw::hit(&l, x, y) {
                Some(i) => Some(i),
                // Keep the card open while the pointer is inside it; otherwise
                // moving towards the card would close it and shrink the window
                // out from under the pointer.
                None if l.card.map(|c| c.contains(x, y)).unwrap_or(false) => current,
                None => None,
            };
            if next != current {
                hover.set(next);
                apply(&win, &area_, &*surface, &shared, &hover, &shown, edge, offset);
            }
            glib::Propagation::Stop
        }
    });

    area.connect_leave_notify_event({
        let shared = Arc::clone(&shared);
        let hover = Rc::clone(&hover);
        let win = win.clone();
        let area_ = area.clone();
        let surface = Rc::clone(&surface);
        let shown = Rc::clone(&shown);
        move |_, _| {
            if hover.replace(None).is_some() {
                apply(&win, &area_, &*surface, &shared, &hover, &shown, edge, offset);
            }
            glib::Propagation::Stop
        }
    });

    area.connect_button_press_event({
        let shared = Arc::clone(&shared);
        let hover = Rc::clone(&hover);
        let win = win.clone();
        move |_, ev| {
            let (x, y) = ev.position();
            if ev.button() == 3 {
                menu(&win);
                return glib::Propagation::Stop;
            }
            let (rings, bus) = match shared.lock() {
                Ok(g) => (g.rings.clone(), g.media_bus.clone()),
                Err(_) => return glib::Propagation::Stop,
            };
            let l = draw::layout(&rings, edge, hover.get());
            if let Some(r) = draw::hit(&l, x, y).and_then(|i| rings.get(i)) {
                if r.action == Some(Action::PlayPause) {
                    if let Some(bus) = bus {
                        if let Err(e) = media::play_pause(&bus) {
                            eprintln!("linotch: play/pause failed: {e}");
                        }
                    }
                }
            }
            glib::Propagation::Stop
        }
    });

    win.show_all();
    apply(&win, &area, &*surface, &shared, &hover, &shown, edge, offset);

    // One timer drives everything the UI needs: resize when the ring count changes,
    // repaint for the media progress arc. A second is plenty for both.
    glib::timeout_add_local(Duration::from_secs(1), {
        let win = win.clone();
        let area = area.clone();
        let surface = Rc::clone(&surface);
        let shared = Arc::clone(&shared);
        let hover = Rc::clone(&hover);
        let shown = Rc::clone(&shown);
        move || {
            apply(&win, &area, &*surface, &shared, &hover, &shown, edge, offset);
            area.queue_draw();
            glib::ControlFlow::Continue
        }
    });
}

/// Resize, re-anchor and re-shape the window for the current rings and hover, but
/// only when one of those actually changed — `resize` on every tick makes the
/// compositor re-map a layer surface once a second.
#[allow(clippy::too_many_arguments)]
fn apply(
    win: &gtk::ApplicationWindow,
    area: &gtk::DrawingArea,
    surface: &dyn surface::Surface,
    shared: &Arc<Mutex<Shared>>,
    hover: &Rc<Cell<Option<usize>>>,
    shown: &Rc<Cell<(usize, Option<usize>)>>,
    edge: Edge,
    offset: f64,
) {
    let rings = shared.lock().map(|g| g.rings.clone()).unwrap_or_default();
    let h = hover.get().filter(|i| *i < rings.len());
    let state = (rings.len(), h);
    if shown.get() == state {
        return;
    }
    // Only the ring count changes the window's length along its edge; hover changes
    // depth alone, which the edge anchor absorbs.
    let relength = shown.get().0 != state.0;
    shown.set(state);

    if rings.is_empty() {
        // Nothing to say — an empty panel is worse than no panel.
        win.hide();
        return;
    }
    let l = draw::layout(&rings, edge, h);
    area.set_size_request(l.w as i32, l.h as i32);
    win.resize(l.w as i32, l.h as i32);
    win.show();
    if relength {
        // Re-anchoring reads the window's size back, and right after a resize that
        // read can still be the old one. Doing it on hover as well was what made the
        // rail slide up and down as the pointer crossed the rings.
        surface.place(win, edge, offset);
    }
    input_region(win, &l);
    area.queue_draw();
}

/// Restrict pointer events to where the notch is actually painted. Without this the
/// window's transparent parts — most of it once a card opens — swallow every click
/// meant for the desktop behind them.
fn input_region(win: &gtk::ApplicationWindow, l: &draw::Layout) {
    let Some(gw) = win.window() else { return };
    let to_rect = |r: draw::Rect| {
        cairo::RectangleInt::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
    };
    let region = cairo::Region::create_rectangle(&to_rect(l.rail));
    if let Some(c) = l.card {
        let _ = region.union_rectangle(&to_rect(c));
    }
    gw.input_shape_combine_region(&region, 0, 0);
}

fn menu(win: &gtk::ApplicationWindow) {
    let m = gtk::Menu::new();
    let refresh = gtk::MenuItem::with_label("Refresh now");
    refresh.connect_activate(|_| {
        REFRESH.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    m.append(&refresh);
    m.append(&gtk::SeparatorMenuItem::new());
    let quit = gtk::MenuItem::with_label("Quit linotch");
    quit.connect_activate({
        let win = win.clone();
        move |_| win.close()
    });
    m.append(&quit);
    m.show_all();
    m.popup_easy(3, gtk::current_event_time());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_climbs_and_stops_climbing() {
        assert_eq!(backoff(0), BACKOFF_BASE);
        assert_eq!(backoff(1), BACKOFF_BASE * 2);
        assert_eq!(backoff(3), BACKOFF_BASE * 8);
        // Capped, and never panics however long the outage lasts.
        assert_eq!(backoff(4), BACKOFF_CAP);
        assert_eq!(backoff(u32::MAX), BACKOFF_CAP);
    }

    #[test]
    fn clock_reads_like_a_player() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(74.0), "1:14");
        assert_eq!(clock(3671.0), "1:01:11");
        assert_eq!(clock(-5.0), "0:00");
    }
}
