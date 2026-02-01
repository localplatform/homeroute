use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Priorité d'un service, détermine le comportement de restart
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePriority {
    /// DNS, DHCP, proxy HTTPS — restart immédiat, retries illimités
    Critical,
    /// API, IPv6 RA — restart avec backoff, max 10 retries
    Important,
    /// Analytics, DDNS, monitoring — restart lent, max 3 retries
    Background,
}

impl ServicePriority {
    fn max_retries(self) -> u32 {
        match self {
            Self::Critical => u32::MAX,
            Self::Important => 10,
            Self::Background => 3,
        }
    }

    fn backoff(self, retry: u32) -> Duration {
        match self {
            Self::Critical => Duration::from_millis(100 * retry as u64),
            Self::Important => Duration::from_secs(retry as u64),
            Self::Background => Duration::from_secs(5 * retry as u64),
        }
    }
}

/// Lance un service supervisé dans une tâche tokio
///
/// Le service est redémarré automatiquement en cas de panne ou de panic,
/// selon sa priorité. Les services critiques redémarrent indéfiniment.
pub fn spawn_supervised<F, Fut>(
    name: &'static str,
    priority: ServicePriority,
    factory: F,
) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let factory = Arc::new(factory);
    tokio::spawn(async move {
        let max_retries = priority.max_retries();
        let mut retries: u32 = 0;
        let mut last_restart = Instant::now();

        loop {
            info!("[supervisor] Starting service: {name}");

            let f = Arc::clone(&factory);
            let result = tokio::spawn(async move {
                let fut = f();
                fut.await
            })
            .await;

            match result {
                Ok(Ok(())) => {
                    info!("[supervisor] {name} exited cleanly");
                    break;
                }
                Ok(Err(e)) => {
                    error!("[supervisor] {name} failed: {e:#}");
                }
                Err(join_error) => {
                    error!("[supervisor] {name} task panicked: {join_error}");
                }
            }

            // Reset retry counter si le service a tourné plus de 60s
            if last_restart.elapsed() > Duration::from_secs(60) {
                retries = 0;
            }

            retries = retries.saturating_add(1);

            if retries > max_retries {
                error!(
                    "[supervisor] {name} exceeded max retries ({max_retries}), giving up"
                );
                break;
            }

            let backoff = priority.backoff(retries);
            warn!(
                "[supervisor] {name} restarting in {backoff:?} (attempt {retries}/{max_retries})"
            );
            tokio::time::sleep(backoff).await;
            last_restart = Instant::now();
        }
    })
}
