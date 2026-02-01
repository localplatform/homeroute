use crate::config::EnvConfig;
use crate::events::EventBus;
use std::sync::Arc;

/// État global de l'application, partagé entre tous les services
///
/// Chaque champ est ajouté progressivement au fur et à mesure de la migration.
/// Pour l'instant (Phase 1), seuls env et events sont disponibles.
pub struct AppState {
    /// Configuration d'environnement
    pub env: Arc<EnvConfig>,
    /// Bus d'événements inter-services
    pub events: Arc<EventBus>,
}

impl AppState {
    pub fn new(env: EnvConfig) -> Self {
        Self {
            env: Arc::new(env),
            events: Arc::new(EventBus::new()),
        }
    }
}
