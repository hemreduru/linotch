//! The one thing the notch draws.
//!
//! Everything on screen — a usage limit, a playing track — is a `Ring` with a list
//! of [`Row`]s behind it. Providers and the media source both produce these, so the
//! drawing code never learns what a provider is, and adding a source never touches
//! it.

/// What sits in the middle of a ring.
#[derive(Clone, Debug, PartialEq)]
pub enum Glyph {
    /// An embedded brand mark, painted in `color` (see `icons::brand`).
    Brand {
        asset: &'static str,
        color: (f64, f64, f64),
    },
    /// A media player, by the desktop-entry name it publishes. Drawn as the app's
    /// own themed icon at rest and as transport controls under the pointer, so the
    /// ring says both *what* is playing and *that clicking does something*.
    Player {
        desktop_entry: String,
        playing: bool,
    },
}

/// How much to trust the number in `fraction`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Health {
    /// Fresh reading from the vendor.
    Ok,
    /// Last known reading; the source did not answer this time.
    Stale,
    /// No usable credential — the ring draws its track only.
    NeedsAuth,
    /// The source is installed but has nothing to say yet.
    Idle,
}

/// One line in the hover card: a label, a value, and optionally a bar and a note.
#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    pub value: String,
    /// 0.0–1.0; `None` draws the row without a bar.
    pub bar: Option<f64>,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct Ring {
    /// Card heading ("Claude", "Spotify").
    pub label: String,
    pub rows: Vec<Row>,
    /// Shown under the heading when there is something to explain rather than
    /// measure — signed out, rate limited, a reading gone stale.
    pub note: String,
    /// 0.0–1.0 of the arc to fill.
    pub fraction: f64,
    pub glyph: Glyph,
    pub health: Health,
    /// Media rings are blue and neutral; usage rings grade green → amber → red.
    pub neutral: bool,
    /// A click on this ring does something.
    pub action: Option<Action>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    PlayPause,
}

impl Ring {
    pub fn usage(label: &str, asset: &'static str, color: (f64, f64, f64)) -> Self {
        Ring {
            label: label.into(),
            rows: Vec::new(),
            note: String::new(),
            fraction: 0.0,
            glyph: Glyph::Brand { asset, color },
            health: Health::Idle,
            neutral: false,
            action: None,
        }
    }

    /// Ring colour. Usage grades with load so a glance is enough; media stays blue.
    pub fn color(&self) -> (f64, f64, f64) {
        self.color_for(self.fraction)
    }

    /// The colour a given fraction earns on this ring. Each card row grades on its
    /// own number — one window at 18% next to one at 71% should not share a colour
    /// just because they share a provider.
    pub fn color_for(&self, fraction: f64) -> (f64, f64, f64) {
        if self.neutral {
            return (0.376, 0.647, 0.980); // #60a5fa
        }
        match fraction {
            f if f >= 0.85 => (0.973, 0.443, 0.443), // #f87171
            f if f >= 0.60 => (0.984, 0.749, 0.141), // #fbbf24
            _ => (0.290, 0.871, 0.502),              // #4ade80
        }
    }

    /// Dimmed while the reading is old or absent, so stale never looks live.
    pub fn alpha(&self) -> f64 {
        match self.health {
            Health::Ok => 1.0,
            Health::Stale => 0.5,
            Health::Idle => 0.4,
            Health::NeedsAuth => 0.34,
        }
    }
}
