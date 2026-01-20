//! XERV Kubernetes Operator binary.
//!
//! This binary runs the XERV operator, which manages XervCluster and XervPipeline
//! custom resources in a Kubernetes cluster.

use futures::StreamExt;
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::watcher::Config as WatcherConfig;
use kube::{Api, Client, CustomResourceExt};
use std::sync::Arc;
use std::time::Duration;
use xerv_operator::controller::{ClusterController, ControllerContext, PipelineController};
use xerv_operator::crd::{XervCluster, XervPipeline};
use xerv_operator::error::OperatorError;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("xerv_operator=info".parse()?)
                .add_directive("kube=info".parse()?),
        )
        .init();

    tracing::info!("Starting XERV Kubernetes Operator");

    // Check for CRD generation mode
    if std::env::args().any(|arg| arg == "--generate-crds") {
        generate_crds()?;
        return Ok(());
    }

    // Connect to Kubernetes
    let client = Client::try_default().await?;
    tracing::info!("Connected to Kubernetes cluster");

    // Create shared controller context
    let ctx = Arc::new(ControllerContext::new(client.clone()));

    // Start controllers concurrently
    let cluster_controller = run_cluster_controller(client.clone(), ctx.clone());
    let pipeline_controller = run_pipeline_controller(client.clone(), ctx.clone());

    // Wait for both controllers (they run forever unless there's an error)
    tokio::select! {
        result = cluster_controller => {
            tracing::error!("Cluster controller exited: {:?}", result);
            result?;
        }
        result = pipeline_controller => {
            tracing::error!("Pipeline controller exited: {:?}", result);
            result?;
        }
    }

    Ok(())
}

/// Run the XervCluster controller.
async fn run_cluster_controller(client: Client, ctx: Arc<ControllerContext>) -> anyhow::Result<()> {
    tracing::info!("Starting XervCluster controller");

    let clusters: Api<XervCluster> = Api::all(client.clone());
    let controller = ClusterController::new(ctx.clone());

    Controller::new(clusters, WatcherConfig::default())
        .shutdown_on_signal()
        .run(
            move |cluster, _ctx| {
                let controller = controller.clone();
                async move {
                    match controller.reconcile(cluster).await {
                        Ok(action) => match action {
                            xerv_operator::controller::ReconcileAction::Requeue(duration) => {
                                Ok(Action::requeue(duration))
                            }
                            xerv_operator::controller::ReconcileAction::Done => {
                                Ok(Action::await_change())
                            }
                        },
                        Err(e) => {
                            tracing::error!(error = %e, "Cluster reconciliation error");
                            Ok(Action::requeue(Duration::from_secs(30)))
                        }
                    }
                }
            },
            |_cluster, error: &OperatorError, _ctx| {
                tracing::error!(error = %error, "Cluster controller error");
                Action::requeue(Duration::from_secs(60))
            },
            ctx.clone(),
        )
        .for_each(|result| async move {
            match result {
                Ok((obj, action)) => {
                    tracing::debug!(
                        cluster = %obj.name,
                        ?action,
                        "Reconciled cluster"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "Cluster controller stream error");
                }
            }
        })
        .await;

    Ok(())
}

/// Run the XervPipeline controller.
async fn run_pipeline_controller(
    client: Client,
    ctx: Arc<ControllerContext>,
) -> anyhow::Result<()> {
    tracing::info!("Starting XervPipeline controller");

    let pipelines: Api<XervPipeline> = Api::all(client.clone());
    let controller = PipelineController::new(ctx.clone());

    Controller::new(pipelines, WatcherConfig::default())
        .shutdown_on_signal()
        .run(
            move |pipeline, _ctx| {
                let controller = controller.clone();
                async move {
                    match controller.reconcile(pipeline).await {
                        Ok(action) => match action {
                            xerv_operator::controller::ReconcileAction::Requeue(duration) => {
                                Ok(Action::requeue(duration))
                            }
                            xerv_operator::controller::ReconcileAction::Done => {
                                Ok(Action::await_change())
                            }
                        },
                        Err(e) => {
                            tracing::error!(error = %e, "Pipeline reconciliation error");
                            Ok(Action::requeue(Duration::from_secs(30)))
                        }
                    }
                }
            },
            |_pipeline, error: &OperatorError, _ctx| {
                tracing::error!(error = %error, "Pipeline controller error");
                Action::requeue(Duration::from_secs(60))
            },
            ctx.clone(),
        )
        .for_each(|result| async move {
            match result {
                Ok((obj, action)) => {
                    tracing::debug!(
                        pipeline = %obj.name,
                        ?action,
                        "Reconciled pipeline"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "Pipeline controller stream error");
                }
            }
        })
        .await;

    Ok(())
}

/// Generate CRD YAML files.
fn generate_crds() -> anyhow::Result<()> {
    println!("---");
    println!("{}", serde_yaml::to_string(&XervCluster::crd())?);
    println!("---");
    println!("{}", serde_yaml::to_string(&XervPipeline::crd())?);
    Ok(())
}
