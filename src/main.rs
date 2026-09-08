//! linotch — a usage notch for Linux.
//!
//! A small pill on a screen edge: one ring per coding assistant showing how much of
//! its limit is gone, plus one for whatever is playing, which doubles as a
//! play/pause button.
//!
//! Threading: one worker owns every read (HTTP and D-Bus both block) and publishes
//! into a mutex; GTK only ever paints what it finds there. Nothing blocking runs on
//! the UI thread, which is why the notch keeps redrawing while a provider hangs.

mod draw;
mod media;
mod providers;
mod ring;
mod surface;

use gtk::prelude::*;
use gtk::{gdk, glib};
use providers::Provider;
use ring::{Action, Glyph, Health, Ring};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use surface::Edge;

const POLL_OK: Duration = Duration::from_secs(60);
const POLL_NO_CREDENTIAL: Duration = Duration::from_secs(300);
const BACKOFF_BASE: Duration = Duration::from_secs(60);
const BACKOFF_CAP: Duration = Duration::from_secs(900);

#[derive(Default)]
struct Shared {
    rings: Vec<Ring>,
    /// Bus name of the player the media ring was drawn for, so a click acts on the
    /// player that was on screen and not on whatever started since.
    media_bus: Option<String>,
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
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
    let offset = env_f64("LINOTCH_OFFSET", 0.5);

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
            "{:8} {} — {} ({})",
            "media",
            m.identity,
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
            ring: Ring::usage(p.label(), p.mark()),
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
        for s in &mut slots {
            if now < s.next {
                continue;
            }
            match s.provider.read() {
                Ok(ws) if !ws.is_empty() => {
                    // The ring shows the window closest to its limit — the one that
                    // will actually stop you — while the tooltip lists them all.
                    let worst = ws
                        .iter()
                        .max_by(|a, b| a.used.total_cmp(&b.used))
                        .expect("non-empty");
                    s.ring.fraction = worst.used;
                    s.ring.health = Health::Ok;
                    s.ring.detail = ws
                        .iter()
                        .map(|w| {
                            let when = w
                                .resets_at
                                .map(providers::until)
                                .unwrap_or_else(|| "?".into());
                            format!("{}  {:.0}%  resets {}", w.label, w.used * 100.0, when)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    s.fails = 0;
                    s.next = now + POLL_OK;
                }
                Ok(_) => {
                    s.ring.health = Health::Idle;
                    s.ring.detail = "no limit windows reported".into();
                    s.next = now + POLL_OK;
                }
                Err(e @ (providers::Error::NoCredential | providers::Error::Rejected { .. })) => {
                    s.ring.health = Health::NeedsAuth;
                    s.ring.fraction = 0.0;
                    s.ring.detail = e.to_string();
                    s.next = now + POLL_NO_CREDENTIAL;
                }
                Err(e) => {
                    // Never invent a number: the last reading stays, dimmed, and the
                    // tooltip says why it is old.
                    s.fails = s.fails.saturating_add(1);
                    if s.ring.health == Health::Ok {
                        s.ring.health = Health::Stale;
                    }
                    s.ring.detail = format!("{}\nlast reading kept ({e})", s.ring.detail);
                    s.next = now + backoff(s.fails);
                }
            }
        }

        let m = bus.as_ref().and_then(media::poll);
        let mut rings: Vec<Ring> = slots.iter().map(|s| s.ring.clone()).collect();
        let media_bus = m.as_ref().map(|m| m.bus.clone());
        if let Some(m) = m {
            rings.push(Ring {
                label: m.identity.clone(),
                detail: format!(
                    "{}\n{}\nclick to {}",
                    media::describe(&m),
                    if m.playing { "playing" } else { "paused" },
                    if m.playing { "pause" } else { "play" }
                ),
                // No length published (a stream, most browser tabs) means no honest
                // progress to draw — the bare track says "playing, length unknown".
                fraction: if m.has_progress { m.progress } else { 0.0 },
                glyph: if m.playing { Glyph::Pause } else { Glyph::Play },
                // Paused is a current fact, not an old reading — dimming it would
                // say "this number may be wrong", which is not what is meant.
                health: Health::Ok,
                neutral: true,
                action: Some(Action::PlayPause),
            });
        }

        if let Ok(mut g) = shared.lock() {
            g.rings = rings;
            g.media_bus = media_bus;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn backoff(fails: u32) -> Duration {
    BACKOFF_BASE
        .saturating_mul(1u32 << fails.min(4))
        .min(BACKOFF_CAP)
}

// ---------------------------------------------------------------------- UI

fn build_ui(app: &gtk::Application, shared: Arc<Mutex<Shared>>, edge: Edge, offset: f64) {
    let surface = surface::detect();
    eprintln!("linotch: surface = {}", surface.name());

    let win = gtk::ApplicationWindow::new(app);
    win.set_app_paintable(true);
    win.set_resizable(false);
    // Without an ARGB visual the pill's corners come out black instead of clear.
    if let Some(v) = gtk::prelude::WidgetExt::screen(&win).and_then(|s| s.rgba_visual()) {
        win.set_visual(Some(&v));
    }
    surface.prepare(&win, edge, offset);

    let area = gtk::DrawingArea::new();
    area.add_events(gdk::EventMask::BUTTON_PRESS_MASK | gdk::EventMask::POINTER_MOTION_MASK);
    area.set_has_tooltip(true);
    win.add(&area);

    area.connect_draw({
        let shared = Arc::clone(&shared);
        move |w, cr| {
            let rings = shared.lock().map(|g| g.rings.clone()).unwrap_or_default();
            let l = draw::layout(rings.len(), edge);
            w.set_size_request(l.w as i32, l.h as i32);
            draw::draw(cr, &rings, &l, edge);
            glib::Propagation::Stop
        }
    });

    area.connect_query_tooltip({
        let shared = Arc::clone(&shared);
        move |_, x, y, _keyboard, tip| {
            let rings = shared.lock().map(|g| g.rings.clone()).unwrap_or_default();
            let l = draw::layout(rings.len(), edge);
            match draw::hit(&l, x as f64, y as f64).and_then(|i| rings.get(i)) {
                Some(r) => {
                    tip.set_text(Some(&format!("{}\n{}", r.label, r.detail)));
                    true
                }
                None => false,
            }
        }
    });

    area.connect_button_press_event({
        let shared = Arc::clone(&shared);
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
            let l = draw::layout(rings.len(), edge);
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
    surface.place(&win, edge, offset);

    // One timer drives everything the UI needs: resize when the ring count changes,
    // repaint for the media progress arc. A second is plenty for both.
    let mut shown = usize::MAX;
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let n = shared.lock().map(|g| g.rings.len()).unwrap_or(0);
        if n != shown {
            shown = n;
            if n == 0 {
                // Nothing to say — an empty black pill is worse than no pill.
                win.hide();
            } else {
                let l = draw::layout(n, edge);
                area.set_size_request(l.w as i32, l.h as i32);
                win.resize(l.w as i32, l.h as i32);
                win.show();
                surface.place(&win, edge, offset);
            }
        }
        area.queue_draw();
        glib::ControlFlow::Continue
    });
}

fn menu(win: &gtk::ApplicationWindow) {
    let m = gtk::Menu::new();
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
}
