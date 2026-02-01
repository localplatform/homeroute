use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crate::store::AnalyticsStore;

/// Run hourly aggregation every 5 minutes.
///
/// Aggregates raw traffic_http records from the last hour into the
/// traffic_hourly table, grouped by hour, device, endpoint, and application.
pub async fn run_hourly_aggregation(store: Arc<AnalyticsStore>) {
    info!("Hourly aggregation task started (runs every 5 minutes)");
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await;
        if let Err(e) = store.aggregate_to_hourly() {
            warn!("Hourly aggregation failed: {}", e);
        }
    }
}

/// Run daily aggregation at approximately 00:30 UTC each day.
///
/// Aggregates traffic_hourly records from the last 2 days into the
/// traffic_daily table. Also runs cleanup of old data afterwards.
pub async fn run_daily_aggregation(store: Arc<AnalyticsStore>) {
    info!("Daily aggregation task started (runs at ~00:30 UTC)");
    loop {
        // Calculate sleep duration until next 00:30 UTC
        let now = chrono::Utc::now();
        let tomorrow_0030 = (now.date_naive() + chrono::Duration::days(1))
            .and_hms_opt(0, 30, 0)
            .unwrap()
            .and_utc();
        let sleep_duration = (tomorrow_0030 - now)
            .to_std()
            .unwrap_or(Duration::from_secs(3600));

        tokio::time::sleep(sleep_duration).await;

        if let Err(e) = store.aggregate_to_daily() {
            warn!("Daily aggregation failed: {}", e);
        }
        if let Err(e) = store.cleanup_old_data() {
            warn!("Cleanup failed: {}", e);
        }
    }
}
