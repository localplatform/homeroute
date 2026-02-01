use rustc_hash::FxHashSet;

/// Adblock domain filter using hierarchical matching.
pub struct AdblockEngine {
    blocked: FxHashSet<String>,
    whitelist: FxHashSet<String>,
    domain_count: usize,
}

impl AdblockEngine {
    pub fn new() -> Self {
        Self {
            blocked: FxHashSet::default(),
            whitelist: FxHashSet::default(),
            domain_count: 0,
        }
    }

    /// Replace the blocked domain set
    pub fn set_blocked(&mut self, domains: FxHashSet<String>) {
        self.domain_count = domains.len();
        self.blocked = domains;
    }

    /// Replace the whitelist
    pub fn set_whitelist(&mut self, domains: Vec<String>) {
        self.whitelist = domains
            .into_iter()
            .map(|d| d.to_lowercase())
            .collect();
    }

    /// Check if a domain is blocked (hierarchical matching with whitelist priority).
    pub fn is_blocked(&self, domain: &str) -> bool {
        let domain = domain.to_lowercase();

        // Walk the domain hierarchy: ads.tracker.com → tracker.com → com
        let mut check = domain.as_str();
        loop {
            // Check whitelist first
            if self.whitelist.contains(check) {
                return false;
            }
            // Check blocklist
            if self.blocked.contains(check) {
                return true;
            }
            // Walk up one level
            match check.find('.') {
                Some(pos) => check = &check[pos + 1..],
                None => break,
            }
        }

        false
    }

    /// Search blocked domains containing a query string
    pub fn search(&self, query: &str, limit: usize) -> Vec<String> {
        let query = query.to_lowercase();
        self.blocked
            .iter()
            .filter(|d| d.contains(&query))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn domain_count(&self) -> usize {
        self.domain_count
    }

    pub fn whitelist_domains(&self) -> Vec<String> {
        self.whitelist.iter().cloned().collect()
    }
}

impl Default for AdblockEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_filter() -> AdblockEngine {
        let mut f = AdblockEngine::new();
        let mut blocked = FxHashSet::default();
        blocked.insert("ads.example.com".to_string());
        blocked.insert("tracker.net".to_string());
        blocked.insert("doubleclick.net".to_string());
        f.set_blocked(blocked);
        f.set_whitelist(vec!["allowed.tracker.net".to_string()]);
        f
    }

    #[test]
    fn test_exact_match() {
        let f = make_filter();
        assert!(f.is_blocked("ads.example.com"));
        assert!(f.is_blocked("tracker.net"));
        assert!(!f.is_blocked("example.com"));
    }

    #[test]
    fn test_hierarchical_match() {
        let f = make_filter();
        // sub.doubleclick.net should be blocked because parent doubleclick.net is blocked
        assert!(f.is_blocked("sub.doubleclick.net"));
        assert!(f.is_blocked("deep.sub.doubleclick.net"));
    }

    #[test]
    fn test_whitelist_override() {
        let f = make_filter();
        // allowed.tracker.net is whitelisted even though tracker.net is blocked
        assert!(!f.is_blocked("allowed.tracker.net"));
        // But tracker.net itself is still blocked
        assert!(f.is_blocked("tracker.net"));
    }

    #[test]
    fn test_not_blocked() {
        let f = make_filter();
        assert!(!f.is_blocked("google.com"));
        assert!(!f.is_blocked("github.com"));
    }

    #[test]
    fn test_search() {
        let f = make_filter();
        let results = f.search("double", 10);
        assert!(results.contains(&"doubleclick.net".to_string()));
    }

    #[test]
    fn test_case_insensitive() {
        let f = make_filter();
        assert!(f.is_blocked("ADS.EXAMPLE.COM"));
        assert!(f.is_blocked("Tracker.Net"));
    }
}
