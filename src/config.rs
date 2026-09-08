//! Where the notch was left.
//!
//! Dragging is only worth having if it survives a restart, and the file is small
//! enough that a failed read is never worth reporting — a fresh default is a
//! perfectly good answer to a corrupt config.

use crate::surface::Edge;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// `right` | `left` | `top` | `bottom`
    pub edge: String,
    /// Position along that edge, 0.0–1.0.
    pub offset: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            edge: "right".into(),
            offset: 0.5,
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("linotch").join("config.json"))
}

impl Config {
    /// The environment still wins, so a one-off `LINOTCH_EDGE=top linotch` behaves
    /// as it always did and does not quietly rewrite what was saved.
    pub fn load() -> (Edge, f64) {
        let saved = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Config>(&t).ok())
            .unwrap_or_default();

        let edge = std::env::var("LINOTCH_EDGE")
            .ok()
            .and_then(|s| Edge::parse(&s))
            .or_else(|| Edge::parse(&saved.edge))
            .unwrap_or(Edge::Right);
        let offset = std::env::var("LINOTCH_OFFSET")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(saved.offset)
            .clamp(0.0, 1.0);
        (edge, offset)
    }

    pub fn save(edge: Edge, offset: f64) {
        let Some(p) = path() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
        let cfg = Config {
            edge: edge.name().into(),
            offset,
        };
        if let Ok(t) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(p, t);
        }
    }
}
