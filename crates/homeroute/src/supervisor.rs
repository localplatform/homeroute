use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use hr_common::service_registry::{
    now_millis, ServicePriorityLevel, ServiceState, ServiceStatus, SharedServiceRegistry,
};

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

    fn to_level(self) -> ServicePriorityLevel {
        match self {
            Self::Critical => ServicePriorityLevel::Critical,
            Self::Important => ServicePriorityLevel::Important,
            Self::Background => ServicePriorityLevel::Background,
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
    registry: SharedServiceRegistry,
    factory: F,
) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let factory = Arc::new(factory);
    let level = priority.to_level();
    tokio::spawn(async move {
        let max_retries = priority.max_retries();
        let mut retries: u32 = 0;
        let mut last_restart = Instant::now();

        loop {
            info!("[supervisor] Starting service: {name}");

            // Mark as running
            {
                let mut reg = registry.write().await;
                reg.insert(
                    name.to_string(),
                    ServiceStatus {
                        name: name.to_string(),
                        state: ServiceState::Running,
                        priority: level.clone(),
                        restart_count: retries,
                        last_state_change: now_millis(),
                        error: None,
                    },
                );
            }

            let f = Arc::clone(&factory);
            let result = tokio::spawn(async move {
                let fut = f();
                fut.await
            })
            .await;

            match result {
                Ok(Ok(())) => {
                    info!("[supervisor] {name} exited cleanly");
                    let mut reg = registry.write().await;
                    if let Some(entry) = reg.get_mut(name) {
                        entry.state = ServiceState::Stopped;
                        entry.last_state_change = now_millis();
                    }
                    break;
                }
                Ok(Err(e)) => {
                    let err_msg = format!("{e:#}");
                    error!("[supervisor] {name} failed: {err_msg}");
                    let mut reg = registry.write().await;
                    if let Some(entry) = reg.get_mut(name) {
                        entry.state = ServiceState::Failed;
                        entry.error = Some(err_msg);
                        entry.last_state_change = now_millis();
                    }
                }
                Err(join_error) => {
                    let err_msg = format!("{join_error}");
                    error!("[supervisor] {name} task panicked: {err_msg}");
                    let mut reg = registry.write().await;
                    if let Some(entry) = reg.get_mut(name) {
                        entry.state = ServiceState::Failed;
                        entry.error = Some(err_msg);
                        entry.last_state_change = now_millis();
                    }
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
                let mut reg = registry.write().await;
                if let Some(entry) = reg.get_mut(name) {
                    entry.state = ServiceState::Stopped;
                    entry.last_state_change = now_millis();
                }
                break;
            }

            let backoff = priority.backoff(retries);
            warn!(
                "[supervisor] {name} restarting in {backoff:?} (attempt {retries}/{max_retries})"
            );

            // Update restart count
            {
                let mut reg = registry.write().await;
                if let Some(entry) = reg.get_mut(name) {
                    entry.restart_count = retries;
                }
            }

            tokio::time::sleep(backoff).await;
            last_restart = Instant::now();
        }
    })
}
