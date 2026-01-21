//! Trigger management operations.

use crate::client::Client;
use crate::error::Result;
use crate::types::{TraceId, TriggerInfo, TriggerResponse};
use serde::{Deserialize, Serialize};

/// Response from listing triggers.
#[derive(Debug, Deserialize)]
struct ListTriggersResponse {
    triggers: Vec<TriggerListItem>,
}

#[derive(Debug, Deserialize)]
struct TriggerListItem {
    id: String,
    trigger_type: String,
    enabled: bool,
}

/// Response from firing a trigger.
#[derive(Debug, Deserialize)]
struct FireTriggerResponse {
    trace_id: TraceId,
    pipeline_id: String,
    trigger_id: String,
}

impl Client {
    /// List all triggers for a pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Returns
    ///
    /// Returns a vector of trigger information.
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let triggers = client.list_triggers("my-pipeline").await?;
    /// for trigger in triggers {
    ///     println!("{}: {} (enabled: {})", trigger.id, trigger.trigger_type, trigger.enabled);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_triggers(&self, pipeline_id: &str) -> Result<Vec<TriggerInfo>> {
        let path = format!("pipelines/{}/triggers", pipeline_id);
        let response = self.get(&path).await?;
        let list_response: ListTriggersResponse = self.handle_response(response).await?;

        Ok(list_response
            .triggers
            .into_iter()
            .map(|t| TriggerInfo {
                id: t.id,
                trigger_type: t.trigger_type,
                enabled: t.enabled,
            })
            .collect())
    }

    /// Fire a trigger to start a new trace.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    /// * `trigger_id` - Trigger identifier
    /// * `payload` - Trigger payload (must be JSON-serializable)
    ///
    /// # Returns
    ///
    /// Returns information about the created trace.
    ///
    /// # Errors
    ///
    /// Returns an error if the trigger is not found, the pipeline is not running,
    /// or the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # use serde_json::json;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let payload = json!({
    ///     "user_id": "123",
    ///     "action": "login"
    /// });
    ///
    /// let response = client.fire_trigger("my-pipeline", "webhook-1", &payload).await?;
    /// println!("Started trace: {}", response.trace_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fire_trigger<T: Serialize>(
        &self,
        pipeline_id: &str,
        trigger_id: &str,
        payload: &T,
    ) -> Result<TriggerResponse> {
        let path = format!("pipelines/{}/triggers/{}/fire", pipeline_id, trigger_id);
        let response = self.post(&path, payload).await?;
        let fire_response: FireTriggerResponse = self.handle_response(response).await?;

        Ok(TriggerResponse {
            trace_id: fire_response.trace_id,
            pipeline_id: fire_response.pipeline_id,
            trigger_id: fire_response.trigger_id,
        })
    }

    /// Pause a trigger (prevent it from creating new traces).
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    /// * `trigger_id` - Trigger identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the trigger is not found or the request fails.
    pub async fn pause_trigger(&self, pipeline_id: &str, trigger_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/triggers/{}/pause", pipeline_id, trigger_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }

    /// Resume a paused trigger.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    /// * `trigger_id` - Trigger identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the trigger is not found or the request fails.
    pub async fn resume_trigger(&self, pipeline_id: &str, trigger_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/triggers/{}/resume", pipeline_id, trigger_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }
}
