//! The one thing the notch draws.
//!
//! Everything on screen — a usage limit, a playing track — is a `Ring`. Providers
//! and the media source both produce these, so `draw.rs` never learns what a
//! provider is and adding a source never touches the drawing code.

/// What sits in the middle of a ring.
#[derive(Clone, Debug, PartialEq)]
pub enum Glyph {
    /// One or two characters, drawn as text (provider marks: C, X, U…).
    Text(String),
    /// Media transport state — drawn as a triangle / two bars.
    Play,
    Pause,
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

#[derive(Clone, Debug)]
pub struct Ring {
    /// Tooltip heading ("Claude", "Spotify").
    pub label: String,
    /// Tooltip body — one line per limit window, or the track name.
    pub detail: String,
    /// 0.0–1.0 of the arc to fill.
    pub fraction: f64,
    pub glyph: Glyph,
    pub health: Health,
    /// Media rings are blue and neutral; usage rings grade green → amber → red.
    pub neutral: bool,
    /// A click on this ring does something (media play/pause).
    pub action: Option<Action>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    PlayPause,
}

impl Ring {
    pub fn usage(label: &str, mark: &str) -> Self {
        Ring {
            label: label.into(),
            detail: String::new(),
            fraction: 0.0,
            glyph: Glyph::Text(mark.into()),
            health: Health::Idle,
            neutral: false,
            action: None,
        }
    }

    /// Ring colour. Usage grades with load so a glance is enough; media stays blue.
    pub fn color(&self) -> (f64, f64, f64) {
        if self.neutral {
            return (0.376, 0.647, 0.980); // #60a5fa
        }
        match self.fraction {
            f if f >= 0.85 => (0.973, 0.443, 0.443), // #f87171
            f if f >= 0.60 => (0.984, 0.749, 0.141), // #fbbf24
            _ => (0.290, 0.871, 0.502),              // #4ade80
        }
    }

    /// Dimmed while the reading is old or absent, so stale never looks live.
    pub fn alpha(&self) -> f64 {
        match self.health {
            Health::Ok => 1.0,
            Health::Stale => 0.45,
            Health::Idle => 0.35,
            Health::NeedsAuth => 0.30,
        }
    }
}
