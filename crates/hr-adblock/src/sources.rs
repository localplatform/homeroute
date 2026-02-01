use anyhow::Result;
use rustc_hash::FxHashSet;
use tracing::{info, warn};

use crate::config::AdblockSource;

/// Source download result
pub struct SourceResult {
    pub name: String,
    pub domain_count: usize,
}

/// Download and parse all adblock sources, returning a unified set of blocked domains.
pub async fn download_all(sources: &[AdblockSource]) -> (FxHashSet<String>, Vec<SourceResult>) {
    let mut all_domains = FxHashSet::with_capacity_and_hasher(80_000, Default::default());
    let mut results = Vec::new();

    // Download sources in parallel
    let mut handles = Vec::new();
    for source in sources {
        let source = source.clone();
        handles.push(tokio::spawn(async move {
            download_source(&source).await
        }));
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let source_name = sources[i].name.clone();
        match handle.await {
            Ok(Ok(domains)) => {
                let count = domains.len();
                info!("Adblock source '{}': {} domains", source_name, count);
                results.push(SourceResult {
                    name: source_name,
                    domain_count: count,
                });
                all_domains.extend(domains);
            }
            Ok(Err(e)) => {
                warn!("Failed to download adblock source '{}': {}", source_name, e);
                results.push(SourceResult {
                    name: source_name,
                    domain_count: 0,
                });
            }
            Err(e) => {
                warn!("Task panicked for source '{}': {}", source_name, e);
                results.push(SourceResult {
                    name: source_name,
                    domain_count: 0,
                });
            }
        }
    }

    info!("Total unique blocked domains: {}", all_domains.len());
    (all_domains, results)
}

async fn download_source(source: &AdblockSource) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client.get(&source.url).send().await?;
    let body = response.text().await?;

    let domains = match source.format.as_str() {
        "hosts" => parse_hosts_file(&body),
        "domain_list" => parse_domain_list(&body),
        "dnsmasq" => parse_dnsmasq_format(&body),
        _ => {
            warn!("Unknown format '{}' for source '{}', trying hosts", source.format, source.name);
            parse_hosts_file(&body)
        }
    };

    Ok(domains)
}

/// Parse hosts file format: `0.0.0.0 domain` or `127.0.0.1 domain`
fn parse_hosts_file(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }

            // Must start with 0.0.0.0 or 127.0.0.1
            if parts[0] != "0.0.0.0" && parts[0] != "127.0.0.1" {
                return None;
            }

            let domain = parts[1].to_lowercase();
            if is_valid_domain(&domain) {
                Some(domain)
            } else {
                None
            }
        })
        .collect()
}

/// Parse domain list format: one domain per line
fn parse_domain_list(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim().to_lowercase();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                return None;
            }
            if is_valid_domain(&line) {
                Some(line)
            } else {
                None
            }
        })
        .collect()
}

/// Parse dnsmasq address format: `address=/domain.com/`
fn parse_dnsmasq_format(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("address=/") && line.ends_with('/') {
                let domain = &line[9..line.len() - 1];
                let domain = domain.to_lowercase();
                if !domain.is_empty() && is_valid_domain(&domain) {
                    return Some(domain);
                }
            }
            None
        })
        .collect()
}

fn is_valid_domain(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 253 {
        return false;
    }

    // Filter out localhost, IPs, and invalid entries
    let blocked_prefixes = [
        "localhost",
        "broadcasthost",
        "local",
        "ip6-",
        "0.",
        "127.",
    ];
    for prefix in &blocked_prefixes {
        if domain.starts_with(prefix) {
            return false;
        }
    }

    // Must contain a dot
    if !domain.contains('.') {
        return false;
    }

    // Must start with alphanumeric
    domain
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric())
}

/// Save domains to a binary cache file for fast startup.
pub fn save_cache(domains: &FxHashSet<String>, path: &std::path::Path) -> Result<()> {
    let domains_vec: Vec<&str> = domains.iter().map(|s| s.as_str()).collect();
    let serialized = serde_json::to_vec(&domains_vec)?;
    std::fs::create_dir_all(path.parent().unwrap_or(path))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &serialized)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Load domains from binary cache file.
pub fn load_cache(path: &std::path::Path) -> Result<FxHashSet<String>> {
    let data = std::fs::read(path)?;
    let domains: Vec<String> = serde_json::from_slice(&data)?;
    let set: FxHashSet<String> = domains.into_iter().collect();
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hosts_file() {
        let content = r#"
# Comment
0.0.0.0 ads.example.com
127.0.0.1 tracker.net
0.0.0.0 localhost
# Another comment
0.0.0.0 bad.site.com
"#;
        let domains = parse_hosts_file(content);
        assert!(domains.contains(&"ads.example.com".to_string()));
        assert!(domains.contains(&"tracker.net".to_string()));
        assert!(domains.contains(&"bad.site.com".to_string()));
        assert!(!domains.contains(&"localhost".to_string()));
    }

    #[test]
    fn test_parse_domain_list() {
        let content = "ads.example.com\ntracker.net\n# comment\n\n";
        let domains = parse_domain_list(content);
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn test_parse_dnsmasq_format() {
        let content = "address=/ads.example.com/\naddress=/tracker.net/\n";
        let domains = parse_dnsmasq_format(content);
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn test_valid_domain() {
        assert!(is_valid_domain("example.com"));
        assert!(is_valid_domain("ads.example.com"));
        assert!(!is_valid_domain("localhost"));
        assert!(!is_valid_domain(""));
        assert!(!is_valid_domain("nodot"));
    }
}
