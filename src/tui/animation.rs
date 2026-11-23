use std::time::{Duration, Instant};

const FRAME_TIME: Duration = Duration::from_millis(42);
const INPUT_IDLE: Duration = Duration::from_millis(150);
pub(super) const SWEEP_SPEED: f64 = 18.0;
pub(super) const SWEEP_LEVELS: u8 = 32;

pub(super) struct Cadence {
    next: Instant,
    previous: Instant,
    phase: Duration,
}

impl Cadence {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            next: now + INPUT_IDLE,
            previous: now + (INPUT_IDLE - FRAME_TIME),
            phase: Duration::ZERO,
        }
    }

    pub(super) fn remaining(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    pub(super) fn step(&mut self, now: Instant) -> Option<Duration> {
        now.checked_duration_since(self.next)?;
        // Cap elapsed time so a delayed host cannot jump the highlight or emit catch-up frames.
        self.phase += now.duration_since(self.previous).min(FRAME_TIME * 2);
        self.previous = now;
        self.next = now + FRAME_TIME;
        Some(self.phase)
    }

    pub(super) fn suspend(&mut self, now: Instant) {
        self.next = now + INPUT_IDLE;
        self.previous = self.next - FRAME_TIME;
    }
}

#[derive(Clone, Copy)]
pub(super) struct Sweep {
    length: usize,
    center: f64,
    trail: f64,
    lead: f64,
    peak: f64,
}

impl Sweep {
    pub(super) fn new(phase: Duration, length: usize) -> Self {
        let radius = (length as f64 / 18.0).clamp(3.0, 10.0);
        Self {
            length,
            center: (phase.as_secs_f64() * SWEEP_SPEED).rem_euclid(length as f64),
            trail: radius * 1.2,
            lead: radius * 0.9,
            peak: f64::from(SWEEP_LEVELS) * (radius / 9.5).min(1.0),
        }
    }

    pub(super) fn strength(self, index: usize) -> u8 {
        let length = self.length as f64;
        let delta = (index as f64 - self.center + length / 2.0).rem_euclid(length) - length / 2.0;
        let extent = if delta < 0.0 { self.trail } else { self.lead };
        let ramp = (1.0 - delta.abs() / extent).max(0.0);
        // Smoothstep flattens both ends of the gradient.
        (self.peak * ramp * ramp * (3.0 - 2.0 * ramp)).round() as u8
    }

    pub(super) fn cells(self) -> impl Iterator<Item = (usize, u8)> {
        let first = (self.center - self.trail).ceil() as isize;
        let last = (self.center + self.lead).floor() as isize;
        (first..=last).filter_map(move |cell| {
            let index = cell.rem_euclid(self.length as isize) as usize;
            let strength = self.strength(index);
            (strength != 0).then_some((index, strength))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_uses_a_smooth_but_bounded_cadence() {
        assert!((Duration::from_millis(40)..=Duration::from_millis(50)).contains(&FRAME_TIME));
    }

    #[test]
    fn cadence_keeps_the_sweep_stationary_until_input_is_idle() {
        let start = Instant::now();
        let mut cadence = Cadence::new(start);
        assert_eq!(cadence.remaining(start), INPUT_IDLE);
        assert_eq!(cadence.step(start + INPUT_IDLE), Some(FRAME_TIME));
        for index in 1..=10 {
            let editing = start + INPUT_IDLE + FRAME_TIME * index;
            cadence.suspend(editing);
            assert_eq!(
                cadence.step(editing + INPUT_IDLE - Duration::from_millis(1)),
                None
            );
        }
        let resumed = start + INPUT_IDLE * 2 + FRAME_TIME * 10;
        assert_eq!(cadence.step(resumed), Some(FRAME_TIME * 2));
        assert_eq!(
            cadence.step(resumed + Duration::from_secs(2)),
            Some(FRAME_TIME * 4)
        );
        assert_eq!(cadence.step(resumed + Duration::from_secs(2)), None);
    }
}
