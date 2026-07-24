//! The scene director (Plan 0009 Phase 3): decides when to rotate presets for a
//! hands-off live show. Auto-rotate runs on a MilkDrop-style dwell timer biased
//! toward energy *drops* — a large downward energy shift rotates sooner, a
//! steady passage holds until the max dwell — with manual hotkey overrides.
//!
//! The decision logic is a pure function of the injected `dt` (seconds since the
//! last call, measured by the shell) plus the analysis [`AnalysisFrame`] and the
//! director's own state. It reads no wall clock of its own, so it is fully
//! deterministic and unit-testable (NFR section 6): the shell owns the clock,
//! the director owns the policy.

use lmv_core::dsp::AnalysisFrame;

use crate::config;

/// Time constant (seconds) for the smoothed energy baseline. ~1.5 s means the
/// baseline follows sustained level changes but ignores per-beat spikes, so a
/// genuine section drop stands out against it.
const ENERGY_TAU: f32 = 1.5;
/// A drop fires when the current energy falls below this fraction *under* the
/// baseline (i.e. energy < baseline * (1 - DROP_FRACTION)).
const DROP_FRACTION: f32 = 0.35;
/// The baseline must exceed this before a drop can register, so near-silence
/// noise (baseline ~0) never looks like a drop.
const DROP_FLOOR: f32 = 0.05;

/// How far past the min dwell (as a fraction of the min->max span) the drop bias
/// is gated: an energy drop can only rotate early once the dwell reaches
/// `min + DROP_GATE_FRACTION * (max - min)`. This softens the drop trigger
/// (ADR-0027) so a drop shortly after a rotation can't rapid-fire another; the
/// timer and novelty triggers are unaffected. At the 20/90 default that gate is
/// ~37.5 s. Scaling to the span keeps it sensible for custom dwell configs too.
const DROP_GATE_FRACTION: f32 = 0.25;

/// Novelty score (from the core detector's ~sqrt(2)-at-a-swap scale) that earns
/// a *full* nudge — pulling the steady-passage cap all the way to the min dwell.
/// A tuning constant; the on-rig soak (Phase 6) is where it gets calibrated.
const NOVELTY_REF: f32 = 0.8;

/// Why the director decided to rotate — surfaced for the title/log and asserted
/// by the unit tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// The max dwell elapsed during a steady passage.
    AutoTimer,
    /// A large downward energy shift past the min dwell.
    AutoDrop,
    /// A track-change novelty boundary pulled the cap in past the min dwell.
    AutoBoundary,
    /// The operator forced the next scene.
    Manual,
}

/// Auto-rotate state machine. Construct from config, then drive with `advance`
/// once per rendered frame; layer manual overrides with `force_next` /
/// `toggle_auto`.
#[derive(Debug, Clone)]
pub struct Director {
    /// Whether auto-rotate is currently active.
    auto: bool,
    /// Dwell clamps (seconds); `min <= max` is enforced at construction.
    min_dwell: f32,
    max_dwell: f32,
    /// Seconds accumulated (from injected `dt`) since the last rotation.
    dwell: f32,
    /// Smoothed energy baseline (EMA of bass+mid+treb); `warm` once seeded.
    baseline: f32,
    warm: bool,
    /// Whether the experimental track-change novelty nudge is active.
    track_change: bool,
}

impl Director {
    /// Build a director from the `[rotate]` config, clamping `max >= min` so a
    /// misconfigured pair can't invert the timer.
    pub fn from_config(rotate: &config::Rotate) -> Self {
        let min_dwell = rotate.min_dwell_secs as f32;
        let max_dwell = (rotate.max_dwell_secs as f32).max(min_dwell);
        Self {
            auto: rotate.auto,
            min_dwell,
            max_dwell,
            dwell: 0.0,
            baseline: 0.0,
            warm: false,
            track_change: rotate.track_change,
        }
    }

    /// Whether auto-rotate is on.
    pub fn auto_enabled(&self) -> bool {
        self.auto
    }

    /// Advance the timer by `dt` seconds against this frame's analysis and
    /// decide whether to rotate. Returns `Some(reason)` exactly on the frames a
    /// rotation should happen (the caller then calls `Renderer::cycle_preset`);
    /// the dwell resets internally on each rotation.
    pub fn advance(&mut self, dt: f32, frame: &AnalysisFrame) -> Option<Rotation> {
        let energy = frame.bass + frame.mid + frame.treb;

        // Compare against the *pre-update* baseline so the current (possibly
        // dropped) sample doesn't first drag the baseline down toward itself.
        let was_warm = self.warm;
        let prev_baseline = self.baseline;
        if self.warm {
            // Frame-rate-independent EMA: alpha depends on dt, not frame count.
            let alpha = 1.0 - (-dt / ENERGY_TAU).exp();
            self.baseline += (energy - self.baseline) * alpha;
        } else {
            self.baseline = energy;
            self.warm = true;
        }

        if !self.auto {
            return None;
        }

        self.dwell += dt;
        if self.dwell < self.min_dwell {
            return None;
        }
        // Novelty nudge: an experimental track-change boundary pulls the cap from
        // the max dwell toward the min dwell, so rotation lands sooner near a
        // detected change. It only shortens the wait past the min dwell, so
        // novelty is never the sole trigger (beatmatched blends have no hard
        // edge). Disabled -> nudge is zero, cap stays at the max dwell.
        let nudge = if self.track_change {
            (frame.novelty / NOVELTY_REF).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cap = self.max_dwell - nudge * (self.max_dwell - self.min_dwell);
        if self.dwell >= cap {
            // A cap the nudge pulled in (still below the hard max) is a boundary
            // rotation; reaching the true max is the steady-passage timer.
            let reason = if self.dwell < self.max_dwell {
                Rotation::AutoBoundary
            } else {
                Rotation::AutoTimer
            };
            self.dwell = 0.0;
            return Some(reason);
        }
        // Drop bias (softened, ADR-0027): a large downward shift rotates early,
        // but only once the dwell is well past the min dwell — gated by
        // DROP_GATE_FRACTION of the min->max span — so a drop just after a
        // rotation can't rapid-fire another.
        let drop_gate = self.min_dwell + DROP_GATE_FRACTION * (self.max_dwell - self.min_dwell);
        let dropped = was_warm
            && self.dwell >= drop_gate
            && prev_baseline > DROP_FLOOR
            && energy < prev_baseline * (1.0 - DROP_FRACTION);
        if dropped {
            self.dwell = 0.0;
            return Some(Rotation::AutoDrop);
        }
        None
    }

    /// Force the next scene now (a manual hotkey): resets the dwell so the auto
    /// timer restarts from this moment. Works whether or not auto-rotate is on.
    pub fn force_next(&mut self) -> Rotation {
        self.dwell = 0.0;
        Rotation::Manual
    }

    /// Toggle auto-rotate; returns the new state. Turning it on resets the dwell
    /// so re-enabling can't trigger an immediate surprise rotation.
    pub fn toggle_auto(&mut self) -> bool {
        self.auto = !self.auto;
        if self.auto {
            self.dwell = 0.0;
        }
        self.auto
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose bass/mid/treb sum to `energy` (split evenly). Other fields
    /// don't affect the director.
    fn frame(energy: f32) -> AnalysisFrame {
        let third = energy / 3.0;
        AnalysisFrame {
            bass: third,
            mid: third,
            treb: third,
            ..AnalysisFrame::default()
        }
    }

    /// A frame with a given energy and novelty score.
    fn frame_nov(energy: f32, novelty: f32) -> AnalysisFrame {
        AnalysisFrame {
            novelty,
            ..frame(energy)
        }
    }

    fn make(auto: bool, min: u32, max: u32, track_change: bool) -> Director {
        Director::from_config(&config::Rotate {
            auto,
            min_dwell_secs: min,
            max_dwell_secs: max,
            track_change,
        })
    }

    fn director(auto: bool, min: u32, max: u32) -> Director {
        make(auto, min, max, false)
    }

    #[test]
    fn steady_passage_rotates_at_max_dwell() {
        let mut d = director(true, 20, 90);
        let steady = frame(1.5);
        // No rotation for the first 89 seconds of a steady, high-energy passage.
        for step in 1..90 {
            assert_eq!(d.advance(1.0, &steady), None, "rotated early at {step}s");
        }
        // The 90th second hits the max-dwell cap.
        assert_eq!(d.advance(1.0, &steady), Some(Rotation::AutoTimer));
    }

    #[test]
    fn energy_drop_rotates_earlier_than_the_cap() {
        let mut d = director(true, 20, 90);
        let loud = frame(1.5);
        // Warm the baseline high and pass the softened drop gate (~37.5 s at the
        // 20/90 default) with a steady passage -> no rotation yet.
        for _ in 0..40 {
            assert_eq!(d.advance(1.0, &loud), None);
        }
        // A sharp drop, now past the drop gate, rotates well before the 90 s cap.
        assert_eq!(d.advance(1.0, &frame(0.1)), Some(Rotation::AutoDrop));
    }

    #[test]
    fn drop_before_min_dwell_holds() {
        let mut d = director(true, 20, 90);
        let loud = frame(1.5);
        for _ in 0..10 {
            assert_eq!(d.advance(1.0, &loud), None);
        }
        // A drop at ~11 s is still inside the min dwell: hold, don't rotate.
        assert_eq!(d.advance(1.0, &frame(0.1)), None);
    }

    #[test]
    fn drop_between_min_dwell_and_gate_is_held() {
        // The softened drop gate (ADR-0027): a drop that lands past the min dwell
        // but before the gate (~37.5 s at the 20/90 default) must NOT rotate, so
        // a drop shortly after a rotation can't rapid-fire another.
        let mut d = director(true, 20, 90);
        let loud = frame(1.5);
        // Warm high and settle past the min dwell but short of the drop gate.
        for _ in 0..25 {
            assert_eq!(d.advance(1.0, &loud), None);
        }
        // A sharp drop at ~26 s (past min 20, before gate ~37.5) is held.
        assert_eq!(d.advance(1.0, &frame(0.1)), None);
    }

    #[test]
    fn manual_next_resets_the_dwell() {
        let mut d = director(true, 20, 90);
        let steady = frame(1.5);
        // Approach the cap...
        for _ in 0..89 {
            assert_eq!(d.advance(1.0, &steady), None);
        }
        // ...then force a manual rotation, which resets the countdown.
        assert_eq!(d.force_next(), Rotation::Manual);
        // The very next steady second must NOT rotate (dwell restarted at 0).
        assert_eq!(d.advance(1.0, &steady), None);
    }

    #[test]
    fn auto_off_never_auto_rotates_but_manual_still_works() {
        let mut d = director(false, 8, 40);
        let loud = frame(1.5);
        // Long steady run plus a drop: no automatic rotation while frozen.
        for _ in 0..100 {
            assert_eq!(d.advance(1.0, &loud), None);
        }
        assert_eq!(d.advance(1.0, &frame(0.1)), None);
        // Manual override still fires.
        assert_eq!(d.force_next(), Rotation::Manual);
    }

    #[test]
    fn default_config_holds_one_scene_but_manual_overrides_work() {
        // ADR-0027: a fresh install (default config) holds one scene — auto is
        // off, so no automatic rotation ever fires, even through a long steady
        // run and a sharp drop.
        let mut d = Director::from_config(&config::Rotate::default());
        assert!(!d.auto_enabled());
        let loud = frame(1.5);
        for _ in 0..200 {
            assert_eq!(d.advance(1.0, &loud), None);
        }
        assert_eq!(d.advance(1.0, &frame(0.1)), None);
        // But the manual next-scene hotkey still fires...
        assert_eq!(d.force_next(), Rotation::Manual);
        // ...and toggling auto on enables rotation live.
        assert!(d.toggle_auto());
        assert!(d.auto_enabled());
    }

    #[test]
    fn toggle_auto_flips_and_reports_state() {
        let mut d = director(true, 8, 40);
        assert!(d.auto_enabled());
        assert!(!d.toggle_auto());
        assert!(!d.auto_enabled());
        assert!(d.toggle_auto());
        assert!(d.auto_enabled());
    }

    #[test]
    fn novelty_boundary_rotates_before_the_cap() {
        let mut d = make(true, 20, 90, true);
        // Steady, no novelty, past the min dwell: still holds toward the cap.
        for _ in 0..25 {
            assert_eq!(d.advance(1.0, &frame_nov(1.0, 0.0)), None);
        }
        // A strong novelty boundary pulls the cap to the min dwell and rotates.
        assert_eq!(
            d.advance(1.0, &frame_nov(1.0, 1.0)),
            Some(Rotation::AutoBoundary)
        );
    }

    #[test]
    fn novelty_before_min_dwell_holds() {
        let mut d = make(true, 20, 90, true);
        for _ in 0..10 {
            assert_eq!(d.advance(1.0, &frame_nov(1.0, 0.0)), None);
        }
        // A boundary at ~11 s is still inside the min dwell: novelty is never the
        // sole trigger, so it holds.
        assert_eq!(d.advance(1.0, &frame_nov(1.0, 1.0)), None);
    }

    #[test]
    fn steady_signal_never_rotates_on_novelty() {
        // Nudge enabled, but a steady low-novelty signal only rotates at the
        // hard max-dwell cap, never early.
        let mut d = make(true, 20, 90, true);
        for step in 1..90 {
            assert_eq!(
                d.advance(1.0, &frame_nov(1.0, 0.0)),
                None,
                "rotated early at {step}s"
            );
        }
        assert_eq!(
            d.advance(1.0, &frame_nov(1.0, 0.0)),
            Some(Rotation::AutoTimer)
        );
    }

    #[test]
    fn disabled_track_change_ignores_novelty() {
        // With the nudge off, even a sustained boundary novelty can't rotate
        // before the cap.
        let mut d = make(true, 20, 90, false);
        for step in 1..90 {
            assert_eq!(
                d.advance(1.0, &frame_nov(1.0, 1.0)),
                None,
                "novelty rotated with the nudge disabled at {step}s"
            );
        }
        assert_eq!(
            d.advance(1.0, &frame_nov(1.0, 1.0)),
            Some(Rotation::AutoTimer)
        );
    }

    #[test]
    fn inverted_dwell_config_is_clamped() {
        // max < min: the constructor clamps max up to min, so the timer is a
        // fixed min-dwell rather than an inverted, always-firing one.
        let mut d = director(true, 30, 5);
        let steady = frame(1.0);
        for step in 1..30 {
            assert_eq!(d.advance(1.0, &steady), None, "rotated early at {step}s");
        }
        assert_eq!(d.advance(1.0, &steady), Some(Rotation::AutoTimer));
    }
}
