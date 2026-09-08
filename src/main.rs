//! linotch — a usage notch for Linux.
//!
//! A pill on a screen edge: one ring per coding assistant showing how much of its
//! limit is gone, plus one for whatever is playing, which doubles as a play/pause
//! button. Hovering a ring opens a card beside it.
//!
//! Threading: one worker owns every read (HTTP and D-Bus both block) and publishes
//! into a mutex; GTK only ever paints what it finds there. Nothing blocking runs on
//! the UI thread, which is why the notch keeps redrawing while a provider hangs.

mod config;
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
use std::cell::RefCell;
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

    let (edge, offset) = config::Config::load();
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

/// Spring, not an ease: a critically-under-damped one overshoots slightly on the
/// way in, which is the "pop", and settles without a second bounce.
const STIFFNESS: f64 = 210.0;
const DAMPING: f64 = 22.0;
const FRAME: Duration = Duration::from_millis(16);
/// How far the pointer may travel before a press counts as a drag rather than a
/// click. Below this a shaky click on the media ring would start dragging.
const DRAG_SLOP: f64 = 4.0;

struct Drag {
    /// Where in the window the press landed, along the rail's axis.
    grab: (f64, f64),
    moved: bool,
}

struct Ui {
    edge: Edge,
    offset: f64,
    /// The panel being drawn. Outlives `target` going to zero, so the close
    /// animation has something to draw.
    panel: Option<draw::Open>,
    target: f64,
    t: f64,
    vel: f64,
    /// Highlighted menu row.
    hot: Option<usize>,
    drag: Option<Drag>,
    /// One animation timer at a time.
    animating: bool,
    /// Last applied window size and panel rect. Both matter: resizing on every
    /// frame would have the compositor re-map the layer surface sixty times a
    /// second, and the input region has to follow the panel even when the window
    /// size does not change — a menu and a card are different rectangles inside
    /// the same window, and a stale region makes the larger of the two unclickable.
    shown: ((i32, i32), Option<(i32, i32, i32, i32)>),
}

impl Ui {
    fn open(&mut self, o: draw::Open) {
        if self.panel != Some(o) && self.panel.is_some() {
            // Switching panels re-pops, rather than sliding a card between rings.
            self.t = self.t.min(0.75);
        }
        self.panel = Some(o);
        self.target = 1.0;
        self.hot = None;
    }

    fn close(&mut self) {
        self.target = 0.0;
    }

    /// One spring step. Returns false once it has settled, which ends the timer.
    fn step(&mut self, dt: f64) -> bool {
        let a = -STIFFNESS * (self.t - self.target) - DAMPING * self.vel;
        self.vel += a * dt;
        self.t += self.vel * dt;
        if (self.t - self.target).abs() < 0.002 && self.vel.abs() < 0.02 {
            self.t = self.target;
            self.vel = 0.0;
            if self.target == 0.0 {
                self.panel = None;
                self.hot = None;
            }
            return false;
        }
        true
    }

    /// What `draw` should render: nothing once the close animation has finished.
    fn drawn(&self) -> Option<draw::Open> {
        if self.t > 0.004 { self.panel } else { None }
    }
}

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
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK,
    );
    win.add(&area);

    let ui = Rc::new(RefCell::new(Ui {
        edge,
        offset,
        panel: None,
        target: 0.0,
        t: 0.0,
        vel: 0.0,
        hot: None,
        drag: None,
        animating: false,
        shown: ((0, 0), None),
    }));

    area.connect_draw({
        let shared = Arc::clone(&shared);
        let ui = Rc::clone(&ui);
        move |_, cr| {
            let rings = shared.lock().map(|g| g.rings.clone()).unwrap_or_default();
            let u = ui.borrow();
            let open = u.drawn().filter(|o| o.ring < rings.len().max(1));
            let l = draw::layout(&rings, u.edge, open);
            draw::draw(cr, &rings, &l, u.edge, open, u.t, u.hot);
            glib::Propagation::Stop
        }
    });

    area.connect_motion_notify_event({
        let ctx = Ctx::new(&win, &area, &surface, &shared, &ui);
        move |_, ev| {
            let (x, y) = ev.position();
            let mut u = ctx.ui.borrow_mut();
            if u.drag.is_some() {
                drop(u);
                ctx.drag_to(x, y);
                return glib::Propagation::Stop;
            }

            let rings = ctx.rings();
            let l = draw::layout(&rings, u.edge, u.drawn());
            // A menu stays until it is used or the pointer leaves; hovering rings
            // under an open menu would swap it for a card mid-click.
            let menu_open = matches!(u.panel.map(|o| o.kind), Some(draw::Kind::Menu)) && u.target > 0.0;
            if menu_open {
                let hot = l.panel.and_then(|r| draw::menu_hit(r, x, y));
                if hot != u.hot {
                    u.hot = hot;
                    ctx.area.queue_draw();
                }
                return glib::Propagation::Stop;
            }

            let next = match draw::hit(&l, x, y) {
                Some(i) => Some(draw::Open { ring: i, kind: draw::Kind::Card }),
                // Keep it open while the pointer is inside the panel it opened.
                None if l.panel.map(|r| r.contains(x, y)).unwrap_or(false) => u.panel,
                None => None,
            };
            let changed = match next {
                Some(o) if u.panel != Some(o) || u.target == 0.0 => {
                    u.open(o);
                    true
                }
                None if u.target != 0.0 => {
                    u.close();
                    true
                }
                _ => false,
            };
            drop(u);
            if changed {
                ctx.animate();
            }
            glib::Propagation::Stop
        }
    });

    area.connect_leave_notify_event({
        let ctx = Ctx::new(&win, &area, &surface, &shared, &ui);
        move |_, _| {
            let mut u = ctx.ui.borrow_mut();
            if u.drag.is_none() && u.target != 0.0 {
                u.close();
                drop(u);
                ctx.animate();
            }
            glib::Propagation::Stop
        }
    });

    area.connect_button_press_event({
        let ctx = Ctx::new(&win, &area, &surface, &shared, &ui);
        move |_, ev| {
            let (x, y) = ev.position();
            let rings = ctx.rings();
            let mut u = ctx.ui.borrow_mut();
            let l = draw::layout(&rings, u.edge, u.drawn());

            if ev.button() == 3 {
                let ring = draw::hit(&l, x, y).unwrap_or(0);
                u.open(draw::Open { ring, kind: draw::Kind::Menu });
                drop(u);
                ctx.animate();
                return glib::Propagation::Stop;
            }

            // A click inside an open menu picks a row and nothing else.
            if matches!(u.panel.map(|o| o.kind), Some(draw::Kind::Menu)) && u.target > 0.0 {
                let pick = l.panel.and_then(|r| draw::menu_hit(r, x, y));
                u.close();
                drop(u);
                ctx.animate();
                match pick {
                    Some(0) => REFRESH.store(true, std::sync::atomic::Ordering::Relaxed),
                    Some(1) => ctx.win.close(),
                    _ => {}
                }
                return glib::Propagation::Stop;
            }

            u.drag = Some(Drag { grab: (x, y), moved: false });
            glib::Propagation::Stop
        }
    });

    area.connect_button_release_event({
        let ctx = Ctx::new(&win, &area, &surface, &shared, &ui);
        move |_, ev| {
            let (x, y) = ev.position();
            let Some(drag) = ctx.ui.borrow_mut().drag.take() else {
                return glib::Propagation::Stop;
            };
            if drag.moved {
                let u = ctx.ui.borrow();
                config::Config::save(u.edge, u.offset);
                return glib::Propagation::Stop;
            }
            // Not a drag, so it was a click.
            let rings = ctx.rings();
            let u = ctx.ui.borrow();
            let l = draw::layout(&rings, u.edge, u.drawn());
            let bus = ctx.shared.lock().ok().and_then(|g| g.media_bus.clone());
            let action = draw::hit(&l, x, y).and_then(|i| rings.get(i)).and_then(|r| r.action);
            drop(u);
            if action == Some(Action::PlayPause) {
                if let Some(bus) = bus {
                    if let Err(e) = media::play_pause(&bus) {
                        eprintln!("linotch: play/pause failed: {e}");
                    }
                }
            }
            glib::Propagation::Stop
        }
    });

    win.show_all();
    let ctx = Ctx::new(&win, &area, &surface, &shared, &ui);
    ctx.apply();

    // Slow tick: the ring count and the media progress arc. The spring runs on its
    // own timer only while it is moving.
    glib::timeout_add_local(Duration::from_secs(1), move || {
        ctx.apply();
        ctx.area.queue_draw();
        glib::ControlFlow::Continue
    });
}

/// The handful of things every event handler needs, cloned once instead of five
/// times per closure.
#[derive(Clone)]
struct Ctx {
    win: gtk::ApplicationWindow,
    area: gtk::DrawingArea,
    surface: Rc<dyn surface::Surface>,
    shared: Arc<Mutex<Shared>>,
    ui: Rc<RefCell<Ui>>,
}

impl Ctx {
    fn new(
        win: &gtk::ApplicationWindow,
        area: &gtk::DrawingArea,
        surface: &Rc<dyn surface::Surface>,
        shared: &Arc<Mutex<Shared>>,
        ui: &Rc<RefCell<Ui>>,
    ) -> Ctx {
        Ctx {
            win: win.clone(),
            area: area.clone(),
            surface: Rc::clone(surface),
            shared: Arc::clone(shared),
            ui: Rc::clone(ui),
        }
    }

    fn rings(&self) -> Vec<Ring> {
        self.shared.lock().map(|g| g.rings.clone()).unwrap_or_default()
    }

    /// Start the 60 Hz spring timer, unless one is already running.
    fn animate(&self) {
        if self.ui.borrow().animating {
            return;
        }
        self.ui.borrow_mut().animating = true;
        let ctx = self.clone();
        glib::timeout_add_local(FRAME, move || {
            let running = ctx.ui.borrow_mut().step(FRAME.as_secs_f64());
            ctx.apply();
            ctx.area.queue_draw();
            if running {
                glib::ControlFlow::Continue
            } else {
                ctx.ui.borrow_mut().animating = false;
                glib::ControlFlow::Break
            }
        });
    }

    /// Resize, re-anchor and re-shape the window — but only when the ring count or
    /// the open/shut state actually changed.
    fn apply(&self) {
        let rings = self.rings();
        let (open, edge, offset) = {
            let u = self.ui.borrow();
            (u.drawn(), u.edge, u.offset)
        };
        if rings.is_empty() {
            self.win.hide();
            self.ui.borrow_mut().shown = ((0, 0), None);
            return;
        }
        let l = draw::layout(&rings, edge, open);
        let size = (l.w as i32, l.h as i32);
        let panel = l
            .panel
            .map(|r| (r.x as i32, r.y as i32, r.w as i32, r.h as i32));
        let was = self.ui.borrow().shown;
        if was == (size, panel) {
            return;
        }
        self.ui.borrow_mut().shown = (size, panel);

        if was.0 != size {
            self.area.set_size_request(size.0, size.1);
            self.win.resize(size.0, size.1);
            self.win.show();
            // Only the length along the edge needs re-anchoring; a change in depth
            // the edge anchor absorbs on its own. And re-anchoring reads the
            // window's size back, which right after a resize can still be the old
            // one — so it is asked for as rarely as possible.
            let along = |s: (i32, i32)| if edge.vertical() { s.1 } else { s.0 };
            if along(was.0) != along(size) {
                self.surface.place(&self.win, edge, offset);
            }
        }
        input_region(&self.win, &l);
        self.area.queue_draw();
    }

    /// Drag the notch. The pointer position is window-relative, so this works in
    /// deltas: nudge the offset, let the window move, and the next event arrives
    /// that much closer to the grab point again. Crossing to a different edge is
    /// decided from the pointer's position on the *screen*, which the current
    /// anchor and margin are enough to reconstruct.
    fn drag_to(&self, x: f64, y: f64) {
        let rings = self.rings();
        let Some(mon) = surface::monitor_geometry(&self.win) else { return };
        let (mw, mh) = (mon.width() as f64, mon.height() as f64);

        let (edge, offset, grab, moved) = {
            let u = self.ui.borrow();
            let Some(d) = u.drag.as_ref() else { return };
            (u.edge, u.offset, d.grab, d.moved)
        };
        if !moved && (x - grab.0).hypot(y - grab.1) < DRAG_SLOP {
            return;
        }
        if !moved {
            // Shut any panel the moment a drag starts — instantly, not on the
            // spring. The window is about to be measured to work out where the
            // pointer is on screen, and a panel makes it a different size.
            let mut u = self.ui.borrow_mut();
            u.panel = None;
            u.target = 0.0;
            u.t = 0.0;
            u.vel = 0.0;
            u.hot = None;
            drop(u);
            self.apply();
        }

        let l = draw::layout(&rings, edge, None);
        let (span, own, along, grab_along) = if edge.vertical() {
            (mh, l.h, y, grab.1)
        } else {
            (mw, l.w, x, grab.0)
        };
        let lead = (span * offset - own / 2.0).clamp(0.0, (span - own).max(0.0));

        // Pointer in screen coordinates.
        let (px, py) = if edge.vertical() {
            (if edge == Edge::Right { mw - l.w + x } else { x }, lead + y)
        } else {
            (lead + x, if edge == Edge::Bottom { mh - l.h + y } else { y })
        };

        // Nearest edge wins — that is what makes it a dock rather than a window.
        let near = [
            (px, Edge::Left),
            (mw - px, Edge::Right),
            (py, Edge::Top),
            (mh - py, Edge::Bottom),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, e)| e)
        .unwrap_or(edge);

        let mut u = self.ui.borrow_mut();
        if let Some(d) = u.drag.as_mut() {
            d.moved = true;
        }
        if near != edge {
            // A new edge re-reads the offset from the pointer outright: the grab
            // point was measured along an axis that no longer exists.
            u.edge = near;
            u.offset = (if near.vertical() { py / mh } else { px / mw }).clamp(0.0, 1.0);
            u.panel = None;
            u.target = 0.0;
            u.t = 0.0;
            u.vel = 0.0;
            u.shown = ((0, 0), None);
            let e = u.edge;
            drop(u);
            self.surface.set_edge(&self.win, e);
            self.apply();
        } else {
            u.offset = (offset + (along - grab_along) / span).clamp(0.0, 1.0);
            let (e, o) = (u.edge, u.offset);
            drop(u);
            self.surface.place(&self.win, e, o);
        }
        self.area.queue_draw();
    }
}

/// Restrict pointer events to where the notch is actually painted. Without this the
/// window's transparent parts — most of it once a panel opens — swallow every click
/// meant for the desktop behind them.
fn input_region(win: &gtk::ApplicationWindow, l: &draw::Layout) {
    let Some(gw) = win.window() else { return };
    let to_rect = |r: draw::Rect| {
        cairo::RectangleInt::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
    };
    let region = cairo::Region::create_rectangle(&to_rect(l.rail));
    if let Some(c) = l.panel {
        let _ = region.union_rectangle(&to_rect(c));
    }
    gw.input_shape_combine_region(&region, 0, 0);
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
