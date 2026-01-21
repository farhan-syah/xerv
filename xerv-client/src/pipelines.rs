//! Pipeline management operations.

use crate::client::Client;
use crate::error::Result;
use crate::types::{PipelineInfo, PipelineStatus};
use serde::Deserialize;

/// Response from deploying a pipeline.
#[derive(Debug, Deserialize)]
struct DeployResponse {
    pipeline_id: String,
    status: String,
}

/// Response from listing pipelines.
#[derive(Debug, Deserialize)]
struct ListResponse {
    pipelines: Vec<PipelineListItem>,
}

#[derive(Debug, Deserialize)]
struct PipelineListItem {
    pipeline_id: String,
    name: String,
    version: String,
    status: String,
    trigger_count: usize,
    node_count: usize,
}

/// Response from getting a single pipeline.
#[derive(Debug, Deserialize)]
struct GetResponse {
    pipeline_id: String,
    name: String,
    version: String,
    status: String,
    trigger_count: usize,
    node_count: usize,
}

impl Client {
    /// Deploy a new pipeline from YAML configuration.
    ///
    /// # Arguments
    ///
    /// * `yaml` - YAML pipeline definition
    ///
    /// # Returns
    ///
    /// Returns the deployed pipeline information.
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment fails or the YAML is invalid.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let yaml = std::fs::read_to_string("my-pipeline.yaml")?;
    /// let pipeline = client.deploy_pipeline(&yaml).await?;
    /// println!("Deployed pipeline: {}", pipeline.pipeline_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn deploy_pipeline(&self, yaml: &str) -> Result<PipelineInfo> {
        let response = self
            .post_raw("pipelines", yaml.as_bytes().to_vec(), "application/x-yaml")
            .await?;

        let deploy_response: DeployResponse = self.handle_response(response).await?;

        Ok(PipelineInfo {
            pipeline_id: deploy_response.pipeline_id,
            name: "".to_string(), // Server doesn't return this in deploy response
            version: "".to_string(),
            status: parse_pipeline_status(&deploy_response.status),
            trigger_count: 0,
            node_count: 0,
        })
    }

    /// List all deployed pipelines.
    ///
    /// # Returns
    ///
    /// Returns a vector of pipeline information.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let pipelines = client.list_pipelines().await?;
    /// for pipeline in pipelines {
    ///     println!("{}: {}", pipeline.pipeline_id, pipeline.name);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_pipelines(&self) -> Result<Vec<PipelineInfo>> {
        let response = self.get("pipelines").await?;
        let list_response: ListResponse = self.handle_response(response).await?;

        Ok(list_response
            .pipelines
            .into_iter()
            .map(|p| PipelineInfo {
                pipeline_id: p.pipeline_id,
                name: p.name,
                version: p.version,
                status: parse_pipeline_status(&p.status),
                trigger_count: p.trigger_count,
                node_count: p.node_count,
            })
            .collect())
    }

    /// Get information about a specific pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Returns
    ///
    /// Returns the pipeline information.
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
    /// let pipeline = client.get_pipeline("my-pipeline").await?;
    /// println!("Pipeline: {} v{}", pipeline.name, pipeline.version);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_pipeline(&self, pipeline_id: &str) -> Result<PipelineInfo> {
        let path = format!("pipelines/{}", pipeline_id);
        let response = self.get(&path).await?;
        let get_response: GetResponse = self.handle_response(response).await?;

        Ok(PipelineInfo {
            pipeline_id: get_response.pipeline_id,
            name: get_response.name,
            version: get_response.version,
            status: parse_pipeline_status(&get_response.status),
            trigger_count: get_response.trigger_count,
            node_count: get_response.node_count,
        })
    }

    /// Delete a pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
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
    /// client.delete_pipeline("my-pipeline").await?;
    /// println!("Pipeline deleted");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_pipeline(&self, pipeline_id: &str) -> Result<()> {
        let path = format!("pipelines/{}", pipeline_id);
        let response = self.delete(&path).await?;
        self.handle_empty_response(response).await
    }

    /// Start a paused or stopped pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    pub async fn start_pipeline(&self, pipeline_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/start", pipeline_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }

    /// Pause a running pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    pub async fn pause_pipeline(&self, pipeline_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/pause", pipeline_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }

    /// Resume a paused pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    pub async fn resume_pipeline(&self, pipeline_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/resume", pipeline_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }

    /// Drain a pipeline (stop accepting new triggers, finish existing traces).
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    pub async fn drain_pipeline(&self, pipeline_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/drain", pipeline_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }

    /// Stop a pipeline immediately.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    pub async fn stop_pipeline(&self, pipeline_id: &str) -> Result<()> {
        let path = format!("pipelines/{}/stop", pipeline_id);
        let response = self.post(&path, &serde_json::json!({})).await?;
        self.handle_empty_response(response).await
    }
}

/// Parse a string status into a PipelineStatus enum.
fn parse_pipeline_status(status: &str) -> PipelineStatus {
    match status.to_lowercase().as_str() {
        "deployed" => PipelineStatus::Deployed,
        "running" => PipelineStatus::Running,
        "paused" => PipelineStatus::Paused,
        "draining" => PipelineStatus::Draining,
        "stopped" => PipelineStatus::Stopped,
        _ => PipelineStatus::Deployed, // Default fallback
    }
}
