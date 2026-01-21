//! Federation-wide status metrics and reporting.
//!
//! This module provides utilities for aggregating cluster deployment statuses
//! into high-level metrics and generating human-readable status messages.
//!
//! # Purpose
//!
//! When deploying pipelines across multiple federated clusters, it's useful to have:
//! - Success rates (what % of clusters are healthy)
//! - Deployment progress (how many clusters are deployed vs synced)
//! - Error patterns (which types of errors are most common)
//!
//! This enables better observability and faster debugging of federation-wide issues.
//!
//! # Example
//!
//! ```ignore
//! use xerv_operator::controller::status_metrics::StatusFormatter;
//!
//! let formatter = StatusFormatter::new();
//! let metrics = formatter.compute_metrics(&cluster_statuses);
//! let message = formatter.format_message(&metrics);
//! // message: "Synced to 3/5 clusters (60.0% success rate) - Most common error: Connection timeout (2x)"
//! ```

use crate::crd::ClusterPipelineStatus;
use std::collections::HashMap;

/// Aggregate metrics for federation-wide deployment health.
///
/// Provides a high-level view of pipeline deployment status across all
/// clusters in a federation.
///
/// # Fields
///
/// - `total_clusters`: Total number of target clusters
/// - `deployed_clusters`: Clusters where deployment succeeded (HTTP 200/201)
/// - `synced_clusters`: Clusters fully in sync (deployed + verified)
/// - `failed_clusters`: Clusters with errors
/// - `success_rate`: Percentage of clusters successfully synced (0-100)
/// - `deployment_rate`: Percentage of clusters with successful deployment (0-100)
/// - `most_common_error`: Most frequently occurring error, if any
#[derive(Debug, Clone, Default)]
pub struct AggregateMetrics {
    /// Total number of target clusters.
    pub total_clusters: usize,
    /// Number of clusters with successful deployment.
    pub deployed_clusters: usize,
    /// Number of clusters in sync.
    pub synced_clusters: usize,
    /// Number of clusters with errors.
    pub failed_clusters: usize,
    /// Success rate as percentage (0-100).
    pub success_rate: f64,
    /// Deployment progress rate as percentage (0-100).
    pub deployment_rate: f64,
    /// Most common error pattern, if any.
    pub most_common_error: Option<String>,
}

/// Status formatter for federation-wide metrics.
///
/// Computes aggregate metrics from individual cluster statuses and
/// formats them into human-readable status messages for display in
/// Kubernetes resource status fields.
///
/// # Design Philosophy
///
/// The formatter provides two levels of information:
/// 1. Quantitative metrics (success rates, counts)
/// 2. Qualitative insights (most common errors, failure patterns)
///
/// This helps operators quickly understand:
/// - Overall health (are most clusters working?)
/// - Progress (deployment vs sync stages)
/// - Actionable errors (what's the most common problem?)
pub struct StatusFormatter;

impl StatusFormatter {
    /// Create a new status formatter.
    pub fn new() -> Self {
        Self
    }

    /// Compute aggregate metrics from cluster statuses.
    ///
    /// Analyzes a list of cluster statuses to produce federation-wide metrics:
    /// - Success rate (% of clusters successfully synced)
    /// - Deployment rate (% of clusters with successful deployment)
    /// - Failure patterns (which errors are most common)
    ///
    /// # Algorithm
    ///
    /// 1. Count deployed/synced/failed clusters
    /// 2. Calculate success and deployment rates
    /// 3. Group errors by prefix (e.g., "Connection timeout", "Auth failed")
    /// 4. Identify most common error pattern
    ///
    /// # Example
    ///
    /// ```ignore
    /// let formatter = StatusFormatter::new();
    /// let metrics = formatter.compute_metrics(&cluster_statuses);
    ///
    /// assert_eq!(metrics.total_clusters, 5);
    /// assert_eq!(metrics.synced_clusters, 3);
    /// assert_eq!(metrics.success_rate, 60.0);
    /// ```
    pub fn compute_metrics(&self, cluster_statuses: &[ClusterPipelineStatus]) -> AggregateMetrics {
        let total = cluster_statuses.len();

        if total == 0 {
            return AggregateMetrics::default();
        }

        let deployed = cluster_statuses.iter().filter(|s| s.deployed).count();
        let synced = cluster_statuses.iter().filter(|s| s.synced).count();
        let failed = cluster_statuses
            .iter()
            .filter(|s| s.error.is_some())
            .count();

        let success_rate = if total > 0 {
            (synced as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let deployment_rate = if total > 0 {
            (deployed as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        // Collect error patterns
        let mut error_patterns: HashMap<String, usize> = HashMap::new();

        for status in cluster_statuses {
            if let Some(ref error) = status.error {
                // Categorize errors by prefix (e.g., "Failed to connect", "Timeout")
                // Split on colon to get the error category
                let error_category = error
                    .split(':')
                    .next()
                    .unwrap_or("Unknown")
                    .trim()
                    .to_string();

                *error_patterns.entry(error_category).or_insert(0) += 1;
            }
        }

        // Find most common error
        let most_common_error = error_patterns
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(error, count)| format!("{} ({}x)", error, count));

        AggregateMetrics {
            total_clusters: total,
            deployed_clusters: deployed,
            synced_clusters: synced,
            failed_clusters: failed,
            success_rate,
            deployment_rate,
            most_common_error,
        }
    }

    /// Build a human-readable status message from aggregate metrics.
    ///
    /// Formats metrics into a concise, actionable status message suitable
    /// for display in Kubernetes resource status fields.
    ///
    /// # Message Formats
    ///
    /// - **Full success**: "All 5 clusters in sync (100% success rate)"
    /// - **Complete failure**: "Deployment failed on all 5 clusters"
    /// - **Partial failure**: "Synced to 3/5 clusters (60.0% success rate) - Most common error: Timeout (2x)"
    /// - **In progress**: "Syncing: 4/5 deployed, 2/5 synced (80.0% complete)"
    ///
    /// # Example
    ///
    /// ```ignore
    /// let formatter = StatusFormatter::new();
    /// let metrics = AggregateMetrics {
    ///     total_clusters: 5,
    ///     synced_clusters: 3,
    ///     failed_clusters: 2,
    ///     success_rate: 60.0,
    ///     most_common_error: Some("Connection timeout (2x)".to_string()),
    ///     ..Default::default()
    /// };
    ///
    /// let message = formatter.format_message(&metrics);
    /// assert_eq!(
    ///     message,
    ///     "Synced to 3/5 clusters (60.0% success rate) - Most common error: Connection timeout (2x)"
    /// );
    /// ```
    pub fn format_message(&self, metrics: &AggregateMetrics) -> String {
        if metrics.synced_clusters == metrics.total_clusters {
            // Perfect success: all clusters are synced
            format!(
                "All {} clusters in sync (100% success rate)",
                metrics.total_clusters
            )
        } else if metrics.deployed_clusters == 0 {
            // Total failure: deployment failed on all clusters
            format!(
                "Deployment failed on all {} clusters",
                metrics.total_clusters
            )
        } else if metrics.failed_clusters > 0 {
            // Partial failure: some clusters failed
            let mut msg = format!(
                "Synced to {}/{} clusters ({:.1}% success rate)",
                metrics.synced_clusters, metrics.total_clusters, metrics.success_rate
            );

            // Add most common error for debugging context
            if let Some(ref error) = metrics.most_common_error {
                msg.push_str(&format!(" - Most common error: {}", error));
            }

            msg
        } else {
            // In progress: deployments are happening but not all synced yet
            format!(
                "Syncing: {}/{} deployed, {}/{} synced ({:.1}% complete)",
                metrics.deployed_clusters,
                metrics.total_clusters,
                metrics.synced_clusters,
                metrics.total_clusters,
                metrics.deployment_rate
            )
        }
    }
}

impl Default for StatusFormatter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_status(
        cluster: &str,
        deployed: bool,
        synced: bool,
        error: Option<&str>,
    ) -> ClusterPipelineStatus {
        ClusterPipelineStatus {
            cluster: cluster.to_string(),
            deployed,
            synced,
            generation: 1,
            status: if synced { "Synced" } else { "Pending" }.to_string(),
            active_traces: 0,
            last_deployed: None,
            error: error.map(|e| e.to_string()),
        }
    }

    #[test]
    fn compute_metrics_all_synced() {
        let formatter = StatusFormatter::new();
        let statuses = vec![
            mock_status("cluster1", true, true, None),
            mock_status("cluster2", true, true, None),
            mock_status("cluster3", true, true, None),
        ];

        let metrics = formatter.compute_metrics(&statuses);

        assert_eq!(metrics.total_clusters, 3);
        assert_eq!(metrics.deployed_clusters, 3);
        assert_eq!(metrics.synced_clusters, 3);
        assert_eq!(metrics.failed_clusters, 0);
        assert_eq!(metrics.success_rate, 100.0);
        assert_eq!(metrics.deployment_rate, 100.0);
        assert_eq!(metrics.most_common_error, None);
    }

    #[test]
    fn compute_metrics_partial_failure() {
        let formatter = StatusFormatter::new();
        let statuses = vec![
            mock_status("cluster1", true, true, None),
            mock_status("cluster2", true, true, None),
            mock_status("cluster3", false, false, Some("Connection timeout")),
            mock_status("cluster4", false, false, Some("Connection timeout")),
            mock_status("cluster5", false, false, Some("Auth failed")),
        ];

        let metrics = formatter.compute_metrics(&statuses);

        assert_eq!(metrics.total_clusters, 5);
        assert_eq!(metrics.deployed_clusters, 2);
        assert_eq!(metrics.synced_clusters, 2);
        assert_eq!(metrics.failed_clusters, 3);
        assert_eq!(metrics.success_rate, 40.0);
        assert_eq!(metrics.deployment_rate, 40.0);
        assert_eq!(
            metrics.most_common_error,
            Some("Connection timeout (2x)".to_string())
        );
    }

    #[test]
    fn compute_metrics_total_failure() {
        let formatter = StatusFormatter::new();
        let statuses = vec![
            mock_status("cluster1", false, false, Some("Failed")),
            mock_status("cluster2", false, false, Some("Failed")),
        ];

        let metrics = formatter.compute_metrics(&statuses);

        assert_eq!(metrics.deployed_clusters, 0);
        assert_eq!(metrics.synced_clusters, 0);
        assert_eq!(metrics.failed_clusters, 2);
        assert_eq!(metrics.success_rate, 0.0);
    }

    #[test]
    fn compute_metrics_empty_list() {
        let formatter = StatusFormatter::new();
        let statuses: Vec<ClusterPipelineStatus> = vec![];

        let metrics = formatter.compute_metrics(&statuses);

        assert_eq!(metrics.total_clusters, 0);
        assert_eq!(metrics.deployed_clusters, 0);
        assert_eq!(metrics.synced_clusters, 0);
    }

    #[test]
    fn format_message_all_synced() {
        let formatter = StatusFormatter::new();
        let metrics = AggregateMetrics {
            total_clusters: 3,
            deployed_clusters: 3,
            synced_clusters: 3,
            failed_clusters: 0,
            success_rate: 100.0,
            deployment_rate: 100.0,
            most_common_error: None,
        };

        let message = formatter.format_message(&metrics);
        assert_eq!(message, "All 3 clusters in sync (100% success rate)");
    }

    #[test]
    fn format_message_total_failure() {
        let formatter = StatusFormatter::new();
        let metrics = AggregateMetrics {
            total_clusters: 3,
            deployed_clusters: 0,
            synced_clusters: 0,
            failed_clusters: 3,
            success_rate: 0.0,
            deployment_rate: 0.0,
            most_common_error: Some("Connection failed (3x)".to_string()),
        };

        let message = formatter.format_message(&metrics);
        assert_eq!(message, "Deployment failed on all 3 clusters");
    }

    #[test]
    fn format_message_partial_failure() {
        let formatter = StatusFormatter::new();
        let metrics = AggregateMetrics {
            total_clusters: 5,
            deployed_clusters: 3,
            synced_clusters: 3,
            failed_clusters: 2,
            success_rate: 60.0,
            deployment_rate: 60.0,
            most_common_error: Some("Timeout (2x)".to_string()),
        };

        let message = formatter.format_message(&metrics);
        assert_eq!(
            message,
            "Synced to 3/5 clusters (60.0% success rate) - Most common error: Timeout (2x)"
        );
    }

    #[test]
    fn format_message_in_progress() {
        let formatter = StatusFormatter::new();
        let metrics = AggregateMetrics {
            total_clusters: 5,
            deployed_clusters: 4,
            synced_clusters: 2,
            failed_clusters: 0,
            success_rate: 40.0,
            deployment_rate: 80.0,
            most_common_error: None,
        };

        let message = formatter.format_message(&metrics);
        assert_eq!(
            message,
            "Syncing: 4/5 deployed, 2/5 synced (80.0% complete)"
        );
    }

    #[test]
    fn error_categorization_groups_by_prefix() {
        let formatter = StatusFormatter::new();
        let statuses = vec![
            mock_status("c1", false, false, Some("Timeout: connection failed")),
            mock_status("c2", false, false, Some("Timeout: deadline exceeded")),
            mock_status("c3", false, false, Some("Auth: invalid credentials")),
        ];

        let metrics = formatter.compute_metrics(&statuses);

        // "Timeout" appears twice, "Auth" once
        assert_eq!(metrics.most_common_error, Some("Timeout (2x)".to_string()));
    }
}
