//! Memory management and auto-unload for RouteLLM

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

/// Auto-unload task that unloads models after idle timeout
pub fn start_auto_unload_task(
    service: Arc<super::RouteLLMService>,
    idle_timeout_secs: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await; // Check every minute

            let last_access = *service.last_access.read().await;
            if let Some(last) = last_access {
                if last.elapsed().as_secs() > idle_timeout_secs {
                    info!("RouteLLM idle timeout reached, unloading models");
                    service.unload().await;
                }
            }
        }
    })
}
