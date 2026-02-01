use std::time::{Duration, Instant};
use rustc_hash::FxHashMap;
use tokio::sync::RwLock;

use crate::records::{DnsRecord, RecordType};

#[derive(Clone)]
struct CacheEntry {
    records: Vec<DnsRecord>,
    inserted_at: Instant,
    ttl: Duration,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() >= self.ttl
    }

    /// Returns records with adjusted TTL (remaining time)
    fn records_with_remaining_ttl(&self) -> Vec<DnsRecord> {
        let elapsed = self.inserted_at.elapsed().as_secs() as u32;
        self.records
            .iter()
            .map(|r| {
                let mut r = r.clone();
                r.ttl = r.ttl.saturating_sub(elapsed);
                r
            })
            .collect()
    }
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct CacheKey {
    name: String,
    qtype: u16,
}

pub struct DnsCache {
    entries: RwLock<FxHashMap<CacheKey, CacheEntry>>,
    max_size: usize,
}

impl DnsCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: RwLock::new(FxHashMap::with_capacity_and_hasher(
                max_size,
                Default::default(),
            )),
            max_size,
        }
    }

    /// Lookup cached records. Returns None if not found or expired.
    pub async fn get(&self, name: &str, qtype: RecordType) -> Option<Vec<DnsRecord>> {
        let key = CacheKey {
            name: name.to_lowercase(),
            qtype: qtype.to_u16(),
        };

        let entries = self.entries.read().await;
        let entry = entries.get(&key)?;

        if entry.is_expired() {
            return None;
        }

        Some(entry.records_with_remaining_ttl())
    }

    /// Insert records into cache. Uses the minimum TTL from the records.
    pub async fn insert(&self, name: &str, qtype: RecordType, records: &[DnsRecord]) {
        if records.is_empty() {
            return;
        }

        let min_ttl = records.iter().map(|r| r.ttl).min().unwrap_or(60);
        // Don't cache records with TTL 0
        if min_ttl == 0 {
            return;
        }

        let key = CacheKey {
            name: name.to_lowercase(),
            qtype: qtype.to_u16(),
        };

        let entry = CacheEntry {
            records: records.to_vec(),
            inserted_at: Instant::now(),
            ttl: Duration::from_secs(min_ttl as u64),
        };

        let mut entries = self.entries.write().await;

        // Evict expired entries if at capacity
        if entries.len() >= self.max_size {
            entries.retain(|_, v| !v.is_expired());
        }

        // If still at capacity, remove oldest entry
        if entries.len() >= self.max_size {
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest_key);
            }
        }

        entries.insert(key, entry);
    }

    /// Remove expired entries (called periodically)
    pub async fn purge_expired(&self) -> usize {
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|_, v| !v.is_expired());
        before - entries.len()
    }

    pub async fn clear(&self) {
        self.entries.write().await.clear();
    }

    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::DnsRecord;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn test_cache_insert_and_get() {
        let cache = DnsCache::new(100);
        let records = vec![DnsRecord::a("example.com", Ipv4Addr::new(1, 2, 3, 4), 300)];

        cache.insert("example.com", RecordType::A, &records).await;
        let result = cache.get("example.com", RecordType::A).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = DnsCache::new(100);
        let result = cache.get("nonexistent.com", RecordType::A).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_cache_case_insensitive() {
        let cache = DnsCache::new(100);
        let records = vec![DnsRecord::a("Example.COM", Ipv4Addr::new(1, 2, 3, 4), 300)];

        cache.insert("Example.COM", RecordType::A, &records).await;
        let result = cache.get("example.com", RecordType::A).await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let cache = DnsCache::new(2);
        let r1 = vec![DnsRecord::a("a.com", Ipv4Addr::new(1, 1, 1, 1), 300)];
        let r2 = vec![DnsRecord::a("b.com", Ipv4Addr::new(2, 2, 2, 2), 300)];
        let r3 = vec![DnsRecord::a("c.com", Ipv4Addr::new(3, 3, 3, 3), 300)];

        cache.insert("a.com", RecordType::A, &r1).await;
        cache.insert("b.com", RecordType::A, &r2).await;
        cache.insert("c.com", RecordType::A, &r3).await;

        assert!(cache.len().await <= 2);
    }
}
