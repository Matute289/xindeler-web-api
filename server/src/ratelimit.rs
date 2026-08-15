use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Same shape as `xindeler-auth`'s `RateLimiter`, ported deliberately: the
/// Python service this replaces has a rate limiter that never evicts IP
/// keys (unbounded memory growth) and doesn't survive a restart. This one
/// evicts stale subjects on a schedule and caps total tracked subjects.
const MAX_SUBJECTS: usize = 10_000;
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

struct RateLimitEntry {
    window_started: Instant,
    last_seen: Instant,
    count: usize,
}

struct RateLimitState {
    subjects: HashMap<IpAddr, RateLimitEntry>,
    next_cleanup: Instant,
}

pub struct RateLimiter {
    state: Mutex<RateLimitState>,
    max: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn with_limits(max: usize, window: Duration) -> Self {
        let now = Instant::now();
        Self {
            state: Mutex::new(RateLimitState {
                subjects: HashMap::new(),
                next_cleanup: now + CLEANUP_INTERVAL,
            }),
            max,
            window,
        }
    }

    pub fn check(&self, addr: IpAddr) -> bool {
        self.check_at(addr, Instant::now())
    }

    fn check_at(&self, addr: IpAddr, now: Instant) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if now >= state.next_cleanup {
            state
                .subjects
                .retain(|_, entry| now.duration_since(entry.last_seen) < self.window);
            state.next_cleanup = now + CLEANUP_INTERVAL;
        }

        if !state.subjects.contains_key(&addr) && state.subjects.len() >= MAX_SUBJECTS {
            return false;
        }

        let entry = state.subjects.entry(addr).or_insert(RateLimitEntry {
            window_started: now,
            last_seen: now,
            count: 0,
        });
        entry.last_seen = now;

        if now.duration_since(entry.window_started) >= self.window {
            entry.window_started = now;
            entry.count = 0;
        }

        if entry.count >= self.max {
            return false;
        }

        entry.count += 1;
        true
    }

    #[cfg(test)]
    fn event_count(&self, addr: IpAddr) -> usize {
        self.state
            .lock()
            .unwrap()
            .subjects
            .get(&addr)
            .map_or(0, |entry| entry.count)
    }

    #[cfg(test)]
    fn subject_count(&self) -> usize {
        self.state.lock().unwrap().subjects.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{RateLimiter, CLEANUP_INTERVAL, MAX_SUBJECTS};
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, Instant};

    const MAX: usize = 3;
    const WINDOW: Duration = Duration::from_secs(3600);

    #[test]
    fn rejected_requests_keep_constant_state() {
        let limiter = RateLimiter::with_limits(MAX, WINDOW);
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8));

        for _ in 0..1000 {
            limiter.check(ip);
        }

        assert_eq!(limiter.event_count(ip), MAX);
        assert_eq!(limiter.subject_count(), 1);
    }

    #[test]
    fn subjects_are_globally_bounded() {
        let limiter = RateLimiter::with_limits(MAX, WINDOW);

        for host in 0..(MAX_SUBJECTS as u32 + 100) {
            limiter.check(IpAddr::V4(Ipv4Addr::from(host)));
        }

        assert_eq!(limiter.subject_count(), MAX_SUBJECTS);
    }

    #[test]
    fn inactive_subjects_are_evicted() {
        let limiter = RateLimiter::with_limits(MAX, WINDOW);
        let first = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
        let second = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));
        let start = Instant::now();

        limiter.check_at(first, start);
        limiter.check_at(second, start + WINDOW + CLEANUP_INTERVAL);

        assert_eq!(limiter.event_count(first), 0);
        assert_eq!(limiter.subject_count(), 1);
    }

    #[test]
    fn custom_limits_are_honored() {
        let limiter = RateLimiter::with_limits(2, Duration::from_secs(60));
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 20));

        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip));
    }
}
