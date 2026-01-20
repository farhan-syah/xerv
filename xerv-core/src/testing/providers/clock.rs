//! Clock provider for time abstraction.
//!
//! Allows tests to use a mock clock with controllable time, while production
//! code uses the real system clock.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Provider trait for time operations.
///
/// This trait abstracts all time-related operations, allowing tests to
/// control time precisely for deterministic behavior.
pub trait ClockProvider: Send + Sync {
    /// Get a monotonic instant (for measuring durations).
    fn now(&self) -> u64;

    /// Get the current system time as milliseconds since UNIX epoch.
    fn system_time_millis(&self) -> u64;

    /// Sleep for the specified duration.
    ///
    /// In mock implementations, this may return immediately or track
    /// pending sleeps for manual advancement.
    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;

    /// Advance time by the specified duration (mock-only operation).
    ///
    /// Real implementations should do nothing.
    fn advance(&self, duration: Duration);

    /// Check if this is a mock clock.
    fn is_mock(&self) -> bool;
}

/// Real clock that uses system time.
#[derive(Debug, Clone)]
pub struct RealClock {
    start: Instant,
}

impl RealClock {
    /// Create a new real clock.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for RealClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockProvider for RealClock {
    fn now(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }

    fn system_time_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before UNIX epoch")
            .as_millis() as u64
    }

    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            tokio::time::sleep(duration).await;
        })
    }

    fn advance(&self, _duration: Duration) {
        // Real clock cannot be manually advanced
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// Mock clock for testing with controllable time.
///
/// The mock clock starts at a fixed or specified time and only advances
/// when explicitly told to via `advance()`.
pub struct MockClock {
    /// Current time in nanoseconds since start.
    current_nanos: AtomicU64,
    /// System time in milliseconds since UNIX epoch.
    system_time_millis: AtomicU64,
    /// Pending sleeps that should be woken when time is advanced.
    pending_sleeps: Mutex<Vec<PendingSleep>>,
}

struct PendingSleep {
    wake_at_nanos: u64,
    waker: Option<std::task::Waker>,
}

impl MockClock {
    /// Create a mock clock starting at time zero.
    pub fn new() -> Self {
        Self {
            current_nanos: AtomicU64::new(0),
            system_time_millis: AtomicU64::new(0),
            pending_sleeps: Mutex::new(Vec::new()),
        }
    }

    /// Create a mock clock fixed at the specified ISO 8601 time.
    ///
    /// # Example
    ///
    /// ```
    /// use xerv_core::testing::{MockClock, ClockProvider};
    ///
    /// let clock = MockClock::fixed("2024-01-15T10:30:00Z");
    /// assert!(clock.system_time_millis() > 0);
    /// ```
    pub fn fixed(iso_time: &str) -> Self {
        let dt = chrono::DateTime::parse_from_rfc3339(iso_time)
            .expect("Invalid ISO 8601 datetime format");
        let millis = dt.timestamp_millis() as u64;

        Self {
            current_nanos: AtomicU64::new(0),
            system_time_millis: AtomicU64::new(millis),
            pending_sleeps: Mutex::new(Vec::new()),
        }
    }

    /// Create a mock clock starting at the current system time.
    pub fn at_now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before UNIX epoch")
            .as_millis() as u64;

        Self {
            current_nanos: AtomicU64::new(0),
            system_time_millis: AtomicU64::new(millis),
            pending_sleeps: Mutex::new(Vec::new()),
        }
    }

    /// Get the current monotonic time in nanoseconds.
    pub fn current_nanos(&self) -> u64 {
        self.current_nanos.load(Ordering::SeqCst)
    }

    /// Wake any pending sleeps whose deadline has passed.
    fn wake_expired_sleeps(&self, current: u64) {
        let mut sleeps = self.pending_sleeps.lock();
        sleeps.retain_mut(|sleep| {
            if sleep.wake_at_nanos <= current {
                if let Some(waker) = sleep.waker.take() {
                    waker.wake();
                }
                false // Remove from list
            } else {
                true // Keep in list
            }
        });
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockProvider for MockClock {
    fn now(&self) -> u64 {
        self.current_nanos.load(Ordering::SeqCst)
    }

    fn system_time_millis(&self) -> u64 {
        self.system_time_millis.load(Ordering::SeqCst)
    }

    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        let wake_at = self.current_nanos.load(Ordering::SeqCst) + duration.as_nanos() as u64;

        // If already past the deadline, return immediately
        if self.current_nanos.load(Ordering::SeqCst) >= wake_at {
            return Box::pin(std::future::ready(()));
        }

        // Create a future that will complete when time is advanced past wake_at
        let clock_ref = self as *const Self;

        Box::pin(MockSleepFuture {
            wake_at,
            clock: clock_ref,
            registered: false,
        })
    }

    fn advance(&self, duration: Duration) {
        let nanos = duration.as_nanos() as u64;
        let new_time = self.current_nanos.fetch_add(nanos, Ordering::SeqCst) + nanos;

        // Also advance system time
        let millis = duration.as_millis() as u64;
        self.system_time_millis.fetch_add(millis, Ordering::SeqCst);

        // Wake any sleeps that should now complete
        self.wake_expired_sleeps(new_time);
    }

    fn is_mock(&self) -> bool {
        true
    }
}

/// Future returned by MockClock::sleep().
struct MockSleepFuture {
    wake_at: u64,
    clock: *const MockClock,
    registered: bool,
}

// SAFETY: The clock pointer is valid for the lifetime of the test
unsafe impl Send for MockSleepFuture {}

impl std::future::Future for MockSleepFuture {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: clock pointer is valid during test
        let clock = unsafe { &*self.clock };

        let current = clock.current_nanos.load(Ordering::SeqCst);

        if current >= self.wake_at {
            std::task::Poll::Ready(())
        } else {
            // Register for wake-up if not already registered
            if !self.registered {
                let mut sleeps = clock.pending_sleeps.lock();
                sleeps.push(PendingSleep {
                    wake_at_nanos: self.wake_at,
                    waker: Some(cx.waker().clone()),
                });
                self.registered = true;
            } else {
                // Update waker in case it changed
                let mut sleeps = clock.pending_sleeps.lock();
                for sleep in sleeps.iter_mut() {
                    if sleep.wake_at_nanos == self.wake_at {
                        sleep.waker = Some(cx.waker().clone());
                        break;
                    }
                }
            }
            std::task::Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn real_clock_advances() {
        let clock = RealClock::new();
        let t1 = clock.now();
        std::thread::sleep(Duration::from_millis(10));
        let t2 = clock.now();
        assert!(t2 > t1);
    }

    #[test]
    fn mock_clock_fixed_time() {
        let clock = MockClock::fixed("2024-01-15T10:30:00Z");
        let millis = clock.system_time_millis();
        // 2024-01-15T10:30:00Z = 1705314600000 ms since epoch
        assert_eq!(millis, 1705314600000);
    }

    #[test]
    fn mock_clock_does_not_advance_automatically() {
        let clock = MockClock::new();
        let t1 = clock.now();
        std::thread::sleep(Duration::from_millis(10));
        let t2 = clock.now();
        assert_eq!(t1, t2);
    }

    #[test]
    fn mock_clock_advance() {
        let clock = MockClock::new();
        assert_eq!(clock.now(), 0);

        clock.advance(Duration::from_secs(1));
        assert_eq!(clock.now(), 1_000_000_000);

        clock.advance(Duration::from_millis(500));
        assert_eq!(clock.now(), 1_500_000_000);
    }

    #[test]
    fn mock_clock_system_time_advances() {
        let clock = MockClock::fixed("2024-01-15T10:30:00Z");
        let t1 = clock.system_time_millis();

        clock.advance(Duration::from_secs(60));
        let t2 = clock.system_time_millis();

        assert_eq!(t2 - t1, 60_000);
    }

    #[tokio::test]
    async fn mock_clock_sleep_completes_on_advance() {
        let clock = Arc::new(MockClock::new());
        let clock_ref = Arc::clone(&clock);

        // Spawn a task that sleeps
        let handle = tokio::spawn(async move {
            clock_ref.sleep(Duration::from_secs(1)).await;
            true
        });

        // Give the task a moment to start
        tokio::task::yield_now().await;

        // Advance time past the sleep duration
        clock.advance(Duration::from_secs(2));

        // The sleep should now complete
        let result = tokio::time::timeout(Duration::from_millis(100), handle)
            .await
            .expect("Timed out waiting for sleep")
            .expect("Task panicked");

        assert!(result);
    }
}
