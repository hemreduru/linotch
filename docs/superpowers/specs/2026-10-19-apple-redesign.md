# Design Specification: Dynamic Island Morphing & Visual Redesign for `draw.rs`

- **Author**: Antigravity Worker
- **Date**: 2026-10-19
- **Status**: Draft Spec (Target for Next Implementation Phase)
- **Target File**: `src/draw.rs` (with minor spring state hooks in `src/main.rs`)
- **Visual Design Reference**: Apple Dynamic Island / iOS fluid interactive squircle morphology

---

## 1. Executive Summary & Design Read

**Design Read**: *A premium, organic, tactile Dynamic Island-style overlay for Linux desktops. Retains linotch's single-surface performance and bezel hug, while eliminating the disjointed "bubble with separate tail" geometry in favor of a unified, fluidly morphing squircle body with Apple-calibrated spring physics, subtle glass rim lighting, and Cairo-optimized ambient shadows.*

### Key Goals
1. **Unified Surface Morphing**: The notch rail and the card/menu become one continuous topological shape. When expanding on hover, the notch smoothly inflates and expands outward from the hovered ring rather than popping out an adjacent card with a triangular tail.
2. **Apple Spring Physics**: Tune `main.rs`'s spring equations ($\text{stiffness}$, $\text{damping}$) to match iOS 17 Dynamic Island physics ($\zeta \approx 0.82$, slight settling oscillation, zero harsh stops).
3. **Typography & Hierarchy**: Transition to Apple-style human interface typography: tabular figures for metrics, proportional tracking, distinct semantic hierarchy.
4. **Cairo-Native Depth**: Achieve an expensive, luminous look within Cairo's 2D software rendering constraints (gradient rim strokes and multi-tier concentric shadow passes; zero expensive image convolution filters).
5. **Reduced-Motion Compliance**: Query `GtkSettings::gtk-enable-animations` and degrade cleanly to instant fade for accessibility.

---

## 2. Shape Morphing & Continuous Curvature Geometry

### 2.1 The Squircle / Superellipse Model
Current `draw.rs` uses standard circular arcs (`cr.arc`) connected by straight lines. True Apple hardware and Dynamic Island surfaces use **continuous curvature** (G2 continuity / curvature continuous), avoiding noticeable curvature jumps where a straight line meets a circular arc.

In Cairo, a superellipse ($|x/a|^n + |y/b|^n = 1$ with $n \approx 4.5$) can be approximated with 4 cubic Bézier splines per quadrant.
For a normalized quadrant of radius $R$ and corner coordinate $(x_0, y_0)$:
- Control point 1: $(x_0 + R \cdot \kappa_1, y_0)$
- Control point 2: $(x_0 + R, y_0 + R \cdot (1.0 - \kappa_2))$
- With $\kappa_1 \approx 0.55228$ (circle) adjusted to $\kappa_1 \approx 0.68$, $\kappa_2 \approx 0.32$ for an Apple-style continuous curvature squircle.

### 2.2 Topological Morphing: Rail to Island
Currently, `layout(&rings, edge, open)` produces two disconnected/tangent rects: `rail` and `panel`, connected by a triangular `tail`.

In the new redesign:
```
State t = 0.0 (Idle Rail):
   Screen Bezel Edge
┌───────────────────────┐
│  ( )    ( )    ( )    │  <- Height: 34px (compact, unobtrusive)
└───┬───────────────┬───┘
    ╰───────────────╯
   Inverse Bezel Flares (Curls)

State t = 1.0 (Expanded Island):
   Screen Bezel Edge
┌───────────────────────────────────────────────┐
│                                               │
│  [Claude]  64% Used              Resets in 2h │
│  ========================================     │
│  Sonnet 3.5                      52% Used     │
│  Opus                            12% Used     │
│                                               │
└───────────────────────────────────────────────┘
  Continuous Curvature Squircle (Smooth, unified perimeter)
```

#### Morph Interpolation Formula:
Let $t \in [0.0, 1.0]$ be the animated spring parameter.
- **Bounding Box Interpolation**:
  - $W(t) = W_{\text{rail}} + (W_{\text{expanded}} - W_{\text{rail}}) \cdot f_{\text{size}}(t)$
  - $H(t) = H_{\text{rail}} + (H_{\text{expanded}} - H_{\text{rail}}) \cdot f_{\text{size}}(t)$
  - Position along the edge shifts smoothly so the island expands symmetrically around the hovered ring's center coordinate $C_{\text{ring}}$.
- **Bezel Flare Curl**:
  - $\text{curl}(t) = \text{CURL}_{\text{base}} \cdot (1.0 - t \cdot 0.65)$
  - At $t=0$, flare smoothly anchors into the screen bezel.
  - At $t=1$, flare tightens to emphasize the island's detached pill presence while remaining anchored to the screen border.

---

## 3. Spring Animation & Motion Physics

### 3.1 Physics Calibration
`main.rs` already features a 2nd-order damped harmonic oscillator:
$$\ddot{x} + 2\zeta\omega_0 \dot{x} + \omega_0^2(x - x_{\text{target}}) = 0$$

Currently calibrated to:
- `STIFFNESS` = 300.0
- `DAMPING` = 28.0

#### Recommended Apple Dynamic Island Profile:
- **Expansion (Open)**:
  - `STIFFNESS_EXPAND` = 380.0
  - `DAMPING_EXPAND` = 26.0
  - *Response*: Swift initial explosion ($t$ reaches 0.9 in ~120ms), tiny natural overshoot ($1.04$ max), settling within 280ms.
- **Collapse (Close)**:
  - `STIFFNESS_COLLAPSE` = 420.0
  - `DAMPING_COLLAPSE` = 32.0
  - *Response*: Critically damped snap back ($\zeta \approx 0.95$), no oscillation, clears screen fast (~180ms).

### 3.2 Staggered Content Dissolve
Do not draw card text during the first 30% of expansion.
- $t \in [0.0, 0.35]$: Ring labels fade out ($\alpha_{\text{rings}} = 1.0 - t/0.35$).
- $t \in [0.35, 1.0]$: Expanded card rows and detailed progress bars fade in ($\alpha_{\text{card}} = (t - 0.35) / 0.65$).
- Prevents text jitter and clipping while the window bounds are scaling.

---

## 4. Typography, Colors & Visual Polish

### 4.1 Typography (Pango)
- **Family**: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Inter", "Cantarell", system-ui, sans-serif`
- **Numeral Tracking**: For percentage numbers and usage stats, enable tabular numbers (`tnum` font feature) to prevent horizontal jitter during rapid polling updates:
  ```rust
  let attrs = pango::AttrList::new();
  attrs.insert(pango::AttrFontFeatures::new("tnum=1"));
  ```
- **Type Scale**:
  - Main Percentage Display: 14pt (Semibold, 600 weight)
  - Detail Header / Title: 12pt (Medium, 500 weight)
  - Secondary Row / Timestamp: 10pt (Regular, 400 weight, color: `#98989D`)

### 4.2 Palette & Contrast
Apple Dynamic Island sits against wallpaper/windows. Solid black `#000000` is the base, accented by luminous semantic highlights:

| Token | Hex / RGBA | Role |
|---|---|---|
| `BG_ISLAND` | `#000000` (`alpha: 0.96` to `1.0`) | Primary island body |
| `STROKE_RIM` | `rgba(255, 255, 255, 0.12)` | 1px top/outer glass edge highlight |
| `STROKE_SUBTLE` | `rgba(255, 255, 255, 0.06)` | Inner dividers and secondary rails |
| `TRACK_BASE` | `rgba(255, 255, 255, 0.10)` | Ring and progress bar background |
| `ACCENT_AMPLE` | `#30D158` (iOS Green) | Usage < 50% |
| `ACCENT_WATCH` | `#FFD60A` (iOS Yellow) | Usage 50% - 75% |
| `ACCENT_CRITICAL` | `#FF453A` (iOS Red) | Usage > 75% |
| `TEXT_PRIMARY` | `#FFFFFF` | Primary labels & percentages |
| `TEXT_MUTED` | `#8E8E93` (SF Gray) | Timestamps, limits, subtitles |

---

## 5. Cairo-Native Blur & Ambient Shadows

### 5.1 The Cairo Software Rendering Constraint
Cairo on Linux CPU surfaces does not have GPU shader pipelines or fast hardware blur. Attempting large convolution kernels on every 60Hz frame causes 100% CPU spikes and dropped frames.

### 5.2 The 3-Tier Multi-Pass Ambient Gradient Shadow
Simulate a deep, premium 24px Apple drop shadow using three concentric expanded geometric passes with exponentially decaying alpha:

1. **Contact Shadow** (Sharp, close):
   - Offset: 1px downward
   - Expand: 2px
   - Color: `rgba(0, 0, 0, 0.35)`
2. **Ambient Bloom** (Mid-range):
   - Offset: 3px downward
   - Expand: 8px
   - Color: `rgba(0, 0, 0, 0.18)`
3. **Atmospheric Feather** (Soft outer rim):
   - Offset: 6px downward
   - Expand: 18px
   - Color: `rgba(0, 0, 0, 0.08)`

When drawn before the main body fill, this creates an authentic dark-mode elevation shadow without measurable CPU cost.

### 5.3 Luminous Rim Lighting (Glass Bezel Edge)
Draw a 1px stroke along the squircle boundary using a subtle vertical gradient:
- Top: `rgba(255, 255, 255, 0.22)` (ambient overhead lighting catching the glass rim)
- Bottom: `rgba(255, 255, 255, 0.04)`
- This single element distinguishes generic flat black rectangles from Apple-grade hardware surfaces.

---

## 6. Accessibility & Reduced Motion

### 6.1 GSettings Query
Before initiating spring dynamics, check the system animation preference:
```rust
fn animations_enabled() -> bool {
    gtk::Settings::default()
        .and_then(|s| s.is_gtk_enable_animations())
        .unwrap_or(true)
}
```

### 6.2 Degraded Behavior
- If `gtk-enable-animations` is `false`:
  - Skip spring integration steps.
  - Set $t = t_{\text{target}}$ immediately upon state change.
  - Render static, non-overshooting layout.
  - Preserves full readability and functionality for users prone to vestibular motion triggers.

---

## 7. Concrete Implementation Steps for Next Worker

1. **Step 1: Replace Path Generators in `src/draw.rs`**:
   - Introduce `squircle_path(cr, rect, corner_radius, smoothness)`.
   - Update `notch_path` to interpolate smoothly from rail to expanded island dimensions based on parameter `t: f64`.
   - Remove obsolete `tail_path` function and the triangular tail attachment.
2. **Step 2: Add Glass Rim & Shadow Passes**:
   - Insert `draw_ambient_shadow(cr, rect, t)` before `body_fill()`.
   - Insert `draw_glass_rim(cr, rect, t)` as an overlay stroke with `cairo::LineWidth = 1.0`.
3. **Step 3: Stagger Content Fade in `draw_card` / `draw_menu`**:
   - Scale content alpha by `((t - 0.35) / 0.65).clamp(0.0, 1.0)`.
   - Suppress drawing text when $t < 0.2$.
4. **Step 4: Hook Reduced Motion**:
   - Check `gtk_enable_animations()` in `Ctx::animate()`.
5. **Step 5: Verify Visuals & Test Suite**:
   - Run `cargo test` (ensure layout geometry assertions pass or adapt to new continuous layout).
   - Render preview with `cargo test -- --ignored preview` and inspect output screenshot in `docs/preview.png`.
