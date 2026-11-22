//! Turns mouse-wheel input into whole steps (one per notch), for wheel volume control.
//!
//! With Miru's smooth-scrolling patch a notch arrives as many small slices that shrink
//! over a few hundred milliseconds. A step is taken once half a notch has arrived; the rest
//! of that notch then brings the count back to zero.

use std::time::{Duration, Instant};

/// A pause longer than this drops any partial notch, so the next scroll starts clean.
const IDLE_RESET: Duration = Duration::from_millis(250);
/// Touchpad pixels per step.
const PIXELS_PER_STEP: f32 = 40.;

#[derive(Debug, Default)]
pub struct WheelSteps {
    /// Lines counted towards the next step; below zero after a step while the rest of that
    /// notch is still arriving.
    lines: f32,
    /// Direction of the last step (+1 or -1), to recognise a reversal.
    dir: f32,
    pixels: f32,
    last: Option<Instant>,
}

impl WheelSteps {
    /// Wheel lines (positive = up / away from the user). Returns the steps to apply.
    pub fn lines(&mut self, lines: f32, per_notch: f32, now: Instant) -> i32 {
        self.reset_if_idle(now);
        let per_notch = per_notch.max(0.01);
        // A slice in the same sense as a leftover owed to the last step means the wheel
        // reversed (the smooth scroller drops the rest of a notch when it does): start afresh
        // instead of counting the new notch twice.
        // Forget the old direction too, or every later slice would look like a reversal.
        if lines * self.lines > 0. && self.lines * self.dir < 0. {
            self.lines = 0.;
            self.dir = 0.;
        }
        self.lines += lines;
        let steps = (self.lines / per_notch).round();
        self.lines -= steps * per_notch;
        if steps != 0. {
            self.dir = steps.signum();
        }
        steps as i32
    }

    /// Precise (touchpad) scrolling in pixels.
    pub fn pixels(&mut self, pixels: f32, now: Instant) -> i32 {
        self.reset_if_idle(now);
        self.pixels += pixels;
        let steps = (self.pixels / PIXELS_PER_STEP).trunc();
        self.pixels -= steps * PIXELS_PER_STEP;
        steps as i32
    }

    fn reset_if_idle(&mut self, now: Instant) {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) > IDLE_RESET)
        {
            self.lines = 0.;
            self.pixels = 0.;
            self.dir = 0.;
        }
        self.last = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PER_NOTCH: f32 = 3.;
    const FRAME: Duration = Duration::from_millis(7);

    /// Replays wheel notches the way the smooth-scrolling patch delivers them: each frame
    /// hands over the share of the queued distance that is due, decaying with a 60 ms time
    /// constant, and a notch in the other direction drops what was still queued.
    fn simulate(notches: &[(f32, Duration)]) -> i32 {
        let mut wheel = WheelSteps::default();
        let mut now = Instant::now();
        let mut queued = 0f32;
        let mut total = 0;
        let frame = |queued: &mut f32, now: Instant, wheel: &mut WheelSteps| {
            let share = 1. - (-FRAME.as_secs_f32() / 0.06).exp();
            let mut slice = *queued * share;
            if (*queued - slice).abs() < 0.01 {
                slice = *queued;
            }
            *queued -= slice;
            if slice != 0. {
                wheel.lines(slice, PER_NOTCH, now)
            } else {
                0
            }
        };
        for &(direction, gap) in notches {
            if queued * direction < 0. {
                queued = 0.;
            }
            queued += direction * PER_NOTCH;
            let mut waited = Duration::ZERO;
            while waited < gap {
                now += FRAME;
                waited += FRAME;
                total += frame(&mut queued, now, &mut wheel);
            }
        }
        for _ in 0..200 {
            now += FRAME;
            total += frame(&mut queued, now, &mut wheel);
        }
        total
    }

    #[test]
    fn one_step_per_notch_in_both_directions() {
        let slow = Duration::from_millis(400);
        assert_eq!(simulate(&[(1., slow)]), 1);
        assert_eq!(simulate(&[(-1., slow)]), -1);
        assert_eq!(simulate(&[(1., slow), (1., slow), (1., slow)]), 3);
        assert_eq!(simulate(&[(-1., slow), (-1., slow), (-1., slow)]), -3);
    }

    #[test]
    fn quick_notches_each_count() {
        let quick = Duration::from_millis(40);
        assert_eq!(simulate(&[(1., quick); 5]), 5);
        assert_eq!(simulate(&[(-1., quick); 5]), -5);
    }

    #[test]
    fn reversing_counts_the_new_direction_once() {
        let quick = Duration::from_millis(60);
        // Down, then up straight away: one step down, one step up.
        assert_eq!(simulate(&[(-1., quick), (1., quick)]), 0);
        assert_eq!(simulate(&[(1., quick), (-1., quick), (-1., quick)]), -1);
        // A full notch down, then up: the up notch must still register.
        let slow = Duration::from_millis(400);
        assert_eq!(simulate(&[(-1., slow), (1., slow)]), 0);
        assert_eq!(simulate(&[(-1., slow), (1., slow), (1., slow)]), 1);
    }

    #[test]
    fn unsliced_notches_step_immediately() {
        let mut wheel = WheelSteps::default();
        let now = Instant::now();
        assert_eq!(wheel.lines(3., PER_NOTCH, now), 1);
        assert_eq!(wheel.lines(3., PER_NOTCH, now), 1);
        assert_eq!(wheel.lines(-3., PER_NOTCH, now), -1);
    }
}
