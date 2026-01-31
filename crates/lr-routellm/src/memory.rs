//! Memory management and auto-unload for RouteLLM

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info};

/// Auto-unload task that unloads models after idle timeout
///
/// This task reads the idle timeout from the service on each check,
/// so it respects runtime configuration changes.
pub fn start_auto_unload_task(service: Arc<super::RouteLLMService>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await; // Check every minute

            // Read current timeout setting (respects runtime config changes)
            let idle_timeout_secs = service.get_idle_timeout().await;

            // Skip check if timeout is 0 (disabled)
            if idle_timeout_secs == 0 {
                debug!("Auto-unload disabled (timeout = 0)");
                continue;
            }

            let last_access = *service.last_access.read().await;
            if let Some(last) = last_access {
                let idle_secs = last.elapsed().as_secs();
                if idle_secs > idle_timeout_secs {
                    info!(
                        "RouteLLM idle timeout reached ({}s > {}s), unloading models",
                        idle_secs, idle_timeout_secs
                    );
                    service.unload().await;
                } else {
                    debug!(
                        "RouteLLM idle check: {}s / {}s",
                        idle_secs, idle_timeout_secs
                    );
                }
            }
        }
    })
}
