use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use tracing::{info, warn};

/// A DHCP lease
#[derive(Debug, Clone)]
pub struct Lease {
    pub expiry: u64,
    pub mac: String,
    pub ip: Ipv4Addr,
    pub hostname: Option<String>,
    pub client_id: Option<String>,
}

/// DHCP lease store with indexes for fast lookups
pub struct LeaseStore {
    leases: HashMap<Ipv4Addr, Lease>,
    by_mac: HashMap<String, Ipv4Addr>,
    by_hostname: HashMap<String, Ipv4Addr>,
    file_path: PathBuf,
}

impl LeaseStore {
    pub fn new(file_path: &str) -> Self {
        Self {
            leases: HashMap::new(),
            by_mac: HashMap::new(),
            by_hostname: HashMap::new(),
            file_path: PathBuf::from(file_path),
        }
    }

    /// Load leases from dnsmasq-compatible file format.
    /// Format: <expiry_timestamp> <mac> <ip> <hostname> <client_id>
    pub fn load_from_file(&mut self) -> Result<usize> {
        let path = self.file_path.clone();
        if !path.exists() {
            return Ok(0);
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read leases from {}", path.display()))?;

        self.leases.clear();
        self.by_mac.clear();
        self.by_hostname.clear();

        let mut count = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                warn!("Invalid lease line: {}", line);
                continue;
            }

            let expiry: u64 = match parts[0].parse() {
                Ok(v) => v,
                Err(_) => {
                    warn!("Invalid expiry in lease: {}", line);
                    continue;
                }
            };

            let mac = parts[1].to_lowercase();
            let ip: Ipv4Addr = match parts[2].parse() {
                Ok(v) => v,
                Err(_) => {
                    warn!("Invalid IP in lease: {}", line);
                    continue;
                }
            };

            let hostname = parts.get(3).and_then(|h| {
                if *h == "*" { None } else { Some(h.to_string()) }
            });
            let client_id = parts.get(4).map(|s| s.to_string());

            self.add_lease_inner(Lease {
                expiry,
                mac: mac.clone(),
                ip,
                hostname: hostname.clone(),
                client_id,
            });
            count += 1;
        }

        info!("Loaded {} leases from {}", count, path.display());
        Ok(count)
    }

    /// Save leases to file in dnsmasq-compatible format (atomic write).
    pub fn save_to_file(&self) -> Result<()> {
        let mut lines = Vec::with_capacity(self.leases.len());

        for lease in self.leases.values() {
            let hostname = lease.hostname.as_deref().unwrap_or("*");
            let client_id = lease.client_id.as_deref().unwrap_or("*");
            lines.push(format!(
                "{} {} {} {} {}",
                lease.expiry, lease.mac, lease.ip, hostname, client_id
            ));
        }

        lines.sort(); // Deterministic output

        let content = lines.join("\n") + "\n";
        let tmp_path = self.file_path.with_extension("tmp");
        std::fs::write(&tmp_path, &content)
            .with_context(|| format!("Failed to write leases to {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, &self.file_path)
            .with_context(|| format!("Failed to rename leases to {}", self.file_path.display()))?;

        Ok(())
    }

    fn add_lease_inner(&mut self, lease: Lease) {
        self.by_mac.insert(lease.mac.clone(), lease.ip);
        if let Some(ref hostname) = lease.hostname {
            self.by_hostname
                .insert(hostname.to_lowercase(), lease.ip);
        }
        self.leases.insert(lease.ip, lease);
    }

    /// Add or update a lease
    pub fn add_lease(&mut self, lease: Lease) {
        // If this IP was previously leased to a different MAC, clean up
        // that MAC's stale by_mac index entry.
        if let Some(old_lease) = self.leases.get(&lease.ip) {
            if old_lease.mac != lease.mac {
                self.by_mac.remove(&old_lease.mac);
            }
            if let Some(ref old_hostname) = old_lease.hostname {
                self.by_hostname.remove(&old_hostname.to_lowercase());
            }
        }
        // Remove old IP mapping for this MAC if it changed
        if let Some(old_ip) = self.by_mac.get(&lease.mac) {
            if *old_ip != lease.ip {
                self.leases.remove(old_ip);
            }
        }
        self.add_lease_inner(lease);
    }

    /// Remove a lease by IP
    pub fn remove_lease(&mut self, ip: Ipv4Addr) {
        if let Some(lease) = self.leases.remove(&ip) {
            self.by_mac.remove(&lease.mac);
            if let Some(ref hostname) = lease.hostname {
                self.by_hostname.remove(&hostname.to_lowercase());
            }
        }
    }

    /// Find IP by MAC address
    pub fn find_ip_by_mac(&self, mac: &str) -> Option<Ipv4Addr> {
        self.by_mac.get(&mac.to_lowercase()).copied()
    }

    /// Find IP by hostname (for DNS expand-hosts)
    pub fn find_ip_by_hostname(&self, hostname: &str) -> Option<Ipv4Addr> {
        self.by_hostname.get(&hostname.to_lowercase()).copied()
    }

    /// Find lease by IP
    pub fn get_lease(&self, ip: Ipv4Addr) -> Option<&Lease> {
        self.leases.get(&ip)
    }

    /// Find lease by MAC
    pub fn get_lease_by_mac(&self, mac: &str) -> Option<&Lease> {
        let ip = self.find_ip_by_mac(mac)?;
        self.leases.get(&ip)
    }

    /// Check if an IP is in use (has an active lease)
    pub fn is_ip_in_use(&self, ip: Ipv4Addr) -> bool {
        if let Some(lease) = self.leases.get(&ip) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            lease.expiry > now
        } else {
            false
        }
    }

    /// Get all leases
    pub fn all_leases(&self) -> Vec<&Lease> {
        self.leases.values().collect()
    }

    /// Purge expired leases
    pub fn purge_expired(&mut self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let expired: Vec<Ipv4Addr> = self
            .leases
            .iter()
            .filter(|(_, lease)| lease.expiry <= now)
            .map(|(ip, _)| *ip)
            .collect();

        let count = expired.len();
        for ip in expired {
            self.remove_lease(ip);
        }
        count
    }

    /// Allocate an IP from the DHCP range for a given MAC.
    /// Priority: existing lease -> static lease -> first available in range.
    pub fn allocate_ip(
        &self,
        mac: &str,
        range_start: Ipv4Addr,
        range_end: Ipv4Addr,
        static_leases: &[(String, Ipv4Addr, String)], // (mac, ip, hostname)
    ) -> Option<(Ipv4Addr, Option<String>)> {
        let mac_lower = mac.to_lowercase();

        // 1. Existing lease for this MAC (verify the index is still valid
        //    and the IP is still within the configured range or a static lease)
        if let Some(ip) = self.find_ip_by_mac(&mac_lower) {
            if let Some(lease) = self.leases.get(&ip) {
                if lease.mac == mac_lower {
                    let ip_u32 = u32::from(ip);
                    let in_range = ip_u32 >= u32::from(range_start) && ip_u32 <= u32::from(range_end);
                    let is_static = static_leases.iter().any(|(smac, sip, _)| {
                        smac.to_lowercase() == mac_lower && *sip == ip
                    });
                    if in_range || is_static {
                        return Some((ip, lease.hostname.clone()));
                    }
                    // IP is outside current range — fall through to reallocate.
                }
            }
            // Stale or out-of-range by_mac index.
            // Fall through to static/pool allocation.
        }

        // 2. Static lease for this MAC
        for (smac, sip, shostname) in static_leases {
            if smac.to_lowercase() == mac_lower {
                let hostname = if shostname.is_empty() {
                    None
                } else {
                    Some(shostname.clone())
                };
                return Some((*sip, hostname));
            }
        }

        // 3. First available IP in range
        let start = u32::from(range_start);
        let end = u32::from(range_end);

        for ip_int in start..=end {
            let ip = Ipv4Addr::from(ip_int);
            if !self.is_ip_in_use(ip) {
                // Also check it's not a static lease IP for another MAC
                let is_reserved = static_leases.iter().any(|(_, sip, _)| *sip == ip);
                if !is_reserved {
                    return Some((ip, None));
                }
            }
        }

        None // Pool exhausted
    }
}

impl Default for LeaseStore {
    fn default() -> Self {
        Self::new("/var/lib/server-dashboard/dhcp-leases")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_find_lease() {
        let mut store = LeaseStore::new("/tmp/test-leases");

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        store.add_lease(Lease {
            expiry: now + 3600,
            mac: "aa:bb:cc:dd:ee:ff".to_string(),
            ip: Ipv4Addr::new(10, 0, 0, 50),
            hostname: Some("testhost".to_string()),
            client_id: None,
        });

        assert_eq!(
            store.find_ip_by_mac("AA:BB:CC:DD:EE:FF"),
            Some(Ipv4Addr::new(10, 0, 0, 50))
        );
        assert_eq!(
            store.find_ip_by_hostname("testhost"),
            Some(Ipv4Addr::new(10, 0, 0, 50))
        );
        assert_eq!(
            store.find_ip_by_hostname("TESTHOST"),
            Some(Ipv4Addr::new(10, 0, 0, 50))
        );
    }

    #[test]
    fn test_allocate_ip() {
        let store = LeaseStore::new("/tmp/test-leases");

        let range_start = Ipv4Addr::new(10, 0, 0, 10);
        let range_end = Ipv4Addr::new(10, 0, 0, 20);

        let result = store.allocate_ip("aa:bb:cc:dd:ee:ff", range_start, range_end, &[]);
        assert_eq!(result, Some((Ipv4Addr::new(10, 0, 0, 10), None)));
    }

    #[test]
    fn test_allocate_static_lease() {
        let store = LeaseStore::new("/tmp/test-leases");

        let range_start = Ipv4Addr::new(10, 0, 0, 10);
        let range_end = Ipv4Addr::new(10, 0, 0, 20);
        let statics = vec![(
            "aa:bb:cc:dd:ee:ff".to_string(),
            Ipv4Addr::new(10, 0, 0, 50),
            "myhost".to_string(),
        )];

        let result = store.allocate_ip("aa:bb:cc:dd:ee:ff", range_start, range_end, &statics);
        assert_eq!(
            result,
            Some((Ipv4Addr::new(10, 0, 0, 50), Some("myhost".to_string())))
        );
    }
}
