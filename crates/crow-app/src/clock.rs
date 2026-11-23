//! The clock motion reads. Every animation start and every "how far along is it"
//! sample goes through `clock::now()`, so a test can step motion exactly instead of
//! sleeping and racing the machine's load.
//!
//! In the app this is `Instant::now()`. In tests, once `ui::init_for_tests` has run on
//! the thread, it is the GPUI test executor's clock: frozen until
//! `executor.advance_clock(..)` moves it, and the same clock GPUI's own
//! `with_animation` and springs read. Only motion timing belongs here; throttles,
//! caches and performance measurements keep the real clock.

use std::time::{Duration, Instant};

#[cfg(test)]
thread_local! {
    static TEST_EXECUTOR: std::cell::RefCell<Option<gpui::BackgroundExecutor>> = const { std::cell::RefCell::new(None) };
}

/// The current moment for motion.
#[cfg(not(test))]
pub fn now() -> Instant {
    Instant::now()
}

/// The current moment for motion: the test executor's frozen clock when one is
/// installed on this thread, else the real clock.
#[cfg(test)]
pub fn now() -> Instant {
    TEST_EXECUTOR.with(|slot| slot.borrow().as_ref().map_or_else(Instant::now, |executor| executor.now()))
}

/// How long ago `since` was, on the motion clock.
pub fn elapsed(since: Instant) -> Duration {
    now().saturating_duration_since(since)
}

/// Make this thread's motion clock the test executor's, so `advance_clock` steps it.
#[cfg(test)]
pub fn install_for_tests(executor: gpui::BackgroundExecutor) {
    TEST_EXECUTOR.with(|slot| *slot.borrow_mut() = Some(executor));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_clock_is_frozen_until_the_test_executor_steps_it(cx: &mut gpui::TestAppContext) {
        install_for_tests(cx.executor());
        let start = now();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(now(), start, "real time passing does not move it");
        cx.executor().advance_clock(Duration::from_millis(150));
        assert_eq!(now() - start, Duration::from_millis(150));
        assert_eq!(elapsed(start), Duration::from_millis(150));
        assert_eq!(now(), cx.executor().now(), "one clock, shared with GPUI's animations");
    }

    #[test]
    fn without_a_test_executor_it_is_the_real_clock() {
        // A fresh thread has no executor installed.
        std::thread::spawn(|| {
            let before = Instant::now();
            let sampled = now();
            assert!(sampled >= before && sampled <= Instant::now());
        })
        .join()
        .expect("clock thread");
    }
}
