use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A single cached value with a TTL — used for `/api/status` (game server
/// probe) and `/api/waitlist/count`. Unlike `xindeler-auth`'s `TimedCache`
/// (keyed, many entries, background eviction thread), this only ever holds
/// one value, so a plain `Mutex<Option<...>>` checked on read is enough —
/// no background thread needed.
pub struct TtlCache<T: Clone> {
    inner: Mutex<Option<(Instant, T)>>,
    ttl: Duration,
}

impl<T: Clone> TtlCache<T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(None),
            ttl,
        }
    }

    pub fn get(&self) -> Option<T> {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .filter(|(cached_at, _)| cached_at.elapsed() < self.ttl)
            .map(|(_, value)| value.clone())
    }

    pub fn set(&self, value: T) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some((Instant::now(), value));
    }

    /// Forces the next `get()` to miss — used when a write (a new waitlist
    /// entry) invalidates the cached count immediately, instead of waiting
    /// out the TTL.
    pub fn clear(&self) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::TtlCache;
    use std::time::Duration;

    #[test]
    fn empty_cache_misses() {
        let cache: TtlCache<u32> = TtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.get(), None);
    }

    #[test]
    fn fresh_value_is_returned() {
        let cache = TtlCache::new(Duration::from_secs(60));
        cache.set(42);
        assert_eq!(cache.get(), Some(42));
    }

    #[test]
    fn cleared_value_misses_immediately() {
        let cache = TtlCache::new(Duration::from_secs(60));
        cache.set(42);
        cache.clear();
        assert_eq!(cache.get(), None);
    }

    #[test]
    fn expired_value_misses() {
        let cache = TtlCache::new(Duration::from_millis(10));
        cache.set(42);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(cache.get(), None);
    }
}
