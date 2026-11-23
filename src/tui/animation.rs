use std::time::{Duration, Instant};

const FRAME_TIME: Duration = Duration::from_millis(100);
const CELLS_PER_FRAME: usize = 2;

pub(super) struct Cadence {
    next: Instant,
    phase: usize,
}

impl Cadence {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            next: now + FRAME_TIME,
            phase: 0,
        }
    }

    pub(super) fn remaining(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    pub(super) fn step(&mut self, now: Instant) -> Option<usize> {
        let elapsed = now.checked_duration_since(self.next)?;
        let frames = 1 + (elapsed.as_millis() / FRAME_TIME.as_millis()) as usize;
        self.phase = self.phase.wrapping_add(frames * CELLS_PER_FRAME);
        self.next = now + FRAME_TIME;
        Some(self.phase)
    }

    pub(super) fn suspend(&mut self, now: Instant) {
        self.next = now + FRAME_TIME;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_caps_ticks_and_skips_elapsed_frames_without_a_catchup_loop() {
        let start = Instant::now();
        let mut cadence = Cadence::new(start);
        assert_eq!(cadence.remaining(start), FRAME_TIME);
        assert_eq!(cadence.step(start + Duration::from_millis(99)), None);
        assert_eq!(cadence.step(start + FRAME_TIME), Some(2));
        assert_eq!(cadence.step(start + Duration::from_millis(690)), Some(12));
        let resumed = start + Duration::from_secs(5);
        cadence.suspend(resumed);
        assert_eq!(cadence.step(resumed), None);
        assert_eq!(cadence.step(resumed + FRAME_TIME), Some(14));
    }
}
