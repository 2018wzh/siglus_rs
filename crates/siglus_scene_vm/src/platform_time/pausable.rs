use std::cell::Cell;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::time::{Duration, Instant as WallInstant};

thread_local! {
    static CLOCK: Cell<(Duration, Option<WallInstant>)> = const { Cell::new((Duration::ZERO, None)) };
}

/// Pause this runtime thread's monotonic clock, returning its previous state.
pub fn set_paused(paused: bool) -> bool {
    CLOCK.with(|clock| {
        let (mut excluded, mut start) = clock.get();
        let previously_paused = start.is_some();
        match (paused, start) {
            (true, None) => start = Some(WallInstant::now()),
            (false, Some(at)) => { excluded += at.elapsed(); start = None; }
            _ => {}
        }
        clock.set((excluded, start));
        previously_paused
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(WallInstant);
impl Instant {
    pub fn now() -> Self {
        CLOCK.with(|clock| {
            let (excluded, paused) = clock.get();
            Self(paused.unwrap_or_else(WallInstant::now) - excluded)
        })
    }
    pub fn elapsed(&self) -> Duration { Self::now().saturating_duration_since(*self) }
    pub fn duration_since(&self, earlier: Self) -> Duration { self.0.duration_since(earlier.0) }
    pub fn saturating_duration_since(&self, earlier: Self) -> Duration { self.0.saturating_duration_since(earlier.0) }
    pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> { self.0.checked_duration_since(earlier.0) }
    pub fn checked_add(&self, duration: Duration) -> Option<Self> { self.0.checked_add(duration).map(Self) }
    pub fn checked_sub(&self, duration: Duration) -> Option<Self> { self.0.checked_sub(duration).map(Self) }
}
impl Add<Duration> for Instant { type Output = Self; fn add(self, rhs: Duration) -> Self { Self(self.0 + rhs) } }
impl Sub<Duration> for Instant { type Output = Self; fn sub(self, rhs: Duration) -> Self { Self(self.0 - rhs) } }
impl Sub for Instant { type Output = Duration; fn sub(self, rhs: Self) -> Duration { self.0 - rhs.0 } }
impl AddAssign<Duration> for Instant { fn add_assign(&mut self, rhs: Duration) { self.0 += rhs; } }
impl SubAssign<Duration> for Instant { fn sub_assign(&mut self, rhs: Duration) { self.0 -= rhs; } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suspension_excludes_wall_time_and_resumes_monotonically() {
        let before = Instant::now();
        set_paused(true);
        let paused = Instant::now();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(Instant::now(), paused);
        set_paused(false);
        let resumed = Instant::now();
        assert!(resumed >= paused && paused >= before);
        assert!(resumed.duration_since(paused) < Duration::from_millis(10));
    }
}
