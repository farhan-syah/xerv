//! XERV Operator Webhook Server
//!
//! Validates that the XERV operator only accesses secrets with appropriate labels.
//! This implements defense-in-depth by adding runtime validation beyond RBAC.

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Response, StatusCode};
use k8s_openapi::api::core::v1::Secret;
use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use tokio::fs;
use tracing::{error, info, warn};

/// Default webhook server port (HTTPS)
const DEFAULT_WEBHOOK_PORT: u16 = 8443;

/// Default TLS certificate path (mounted by Kubernetes)
const DEFAULT_TLS_CERT_PATH: &str = "/certs/tls.crt";

/// Default TLS private key path (mounted by Kubernetes)
const DEFAULT_TLS_KEY_PATH: &str = "/certs/tls.key";

/// Allowed secret labels for XERV operator access
const ALLOWED_LABELS: &[(&str, &str)] = &[
    ("app.kubernetes.io/managed-by", "xerv"),
    ("app.kubernetes.io/managed-by", "xerv-operator"),
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("XERV Operator Webhook Server starting...");

    // Load configuration from environment
    let port: u16 = env::var("WEBHOOK_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_WEBHOOK_PORT);

    let tls_cert = env::var("TLS_CERT_FILE").unwrap_or_else(|_| DEFAULT_TLS_CERT_PATH.to_string());
    let tls_key = env::var("TLS_KEY_FILE").unwrap_or_else(|_| DEFAULT_TLS_KEY_PATH.to_string());

    info!(
        port = port,
        tls_cert = %tls_cert,
        tls_key = %tls_key,
        "Webhook configuration loaded"
    );

    // Load TLS certificates
    let cert_pem = fs::read(&tls_cert).await?;
    let key_pem = fs::read(&tls_key).await?;

    info!("TLS certificates loaded successfully");

    // Build TLS acceptor
    let certs = rustls_pemfile::certs(&mut cert_pem.as_slice()).collect::<Result<Vec<_>, _>>()?;

    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())?
        .ok_or("No private key found in PEM file")?;

    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let tls_acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls_config));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!(addr = %addr, "Webhook server listening");

    // Start server with TLS
    let listener = tokio::net::TcpListener::bind(addr).await?;

    loop {
        let (stream, _peer_addr) = listener.accept().await?;
        let acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    let io = hyper_util::rt::TokioIo::new(tls_stream);

                    let service = hyper::service::service_fn(handle_request);

                    if let Err(e) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await
                    {
                        error!(error = %e, "Connection error");
                    }
                }
                Err(e) => {
                    error!(error = %e, "TLS handshake failed");
                }
            }
        });
    }
}

/// Handle incoming HTTP requests
async fn handle_request(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::POST, "/validate/secret") => Ok(validate_secret_access(req).await),
        (&Method::GET, "/healthz") => Ok(health_check()),
        (&Method::GET, "/readyz") => Ok(ready_check()),
        _ => Ok(not_found()),
    }
}

/// Kubernetes admission review structures
#[derive(serde::Deserialize, serde::Serialize)]
struct AdmissionReview {
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<AdmissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<AdmissionResponse>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct AdmissionRequest {
    uid: String,
    operation: String,
    object: Option<Secret>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct AdmissionResponse {
    uid: String,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<AdmissionStatus>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct AdmissionStatus {
    code: i32,
    message: String,
}

/// Validate secret access request
async fn validate_secret_access(req: Request<hyper::body::Incoming>) -> Response<Full<Bytes>> {
    // Parse admission review request
    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!(error = %e, "Failed to read request body");
            return error_response("Failed to read request body");
        }
    };

    let admission_review: AdmissionReview = match serde_json::from_slice(&body_bytes) {
        Ok(review) => review,
        Err(e) => {
            error!(error = %e, "Failed to parse AdmissionReview");
            return error_response("Invalid AdmissionReview format");
        }
    };

    let request = match admission_review.request {
        Some(req) => req,
        None => {
            error!("AdmissionReview missing request");
            return error_response("Missing request in AdmissionReview");
        }
    };

    // Validate the secret access
    let response = validate_secret(&request);

    // Build response
    let review_response = AdmissionReview {
        request: None,
        response: Some(response),
    };

    match serde_json::to_string(&review_response) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(json)))
            .unwrap_or_else(|e| {
                error!(error = %e, "Failed to build admission review response");
                error_response("Failed to build response")
            }),
        Err(e) => {
            error!(error = %e, "Failed to serialize response");
            error_response("Failed to serialize response")
        }
    }
}

/// Validate secret access based on labels
fn validate_secret(req: &AdmissionRequest) -> AdmissionResponse {
    let uid = req.uid.clone();

    // Extract secret from request
    let secret = match &req.object {
        Some(s) => s,
        None => {
            warn!(operation = %req.operation, "No object in admission request");
            // Allow if no object present (list/watch operations)
            return AdmissionResponse {
                uid,
                allowed: true,
                status: None,
            };
        }
    };

    let secret_name = secret.metadata.name.as_deref().unwrap_or("<unnamed>");

    let namespace = secret.metadata.namespace.as_deref().unwrap_or("default");

    info!(
        secret = secret_name,
        namespace = namespace,
        operation = %req.operation,
        "Validating secret access"
    );

    // Check if secret has allowed labels
    let labels = secret.metadata.labels.as_ref();

    let has_allowed_label = labels.is_some_and(|labels| {
        ALLOWED_LABELS
            .iter()
            .any(|(key, value)| labels.get(*key).map(|v| v == *value).unwrap_or(false))
    });

    if has_allowed_label {
        info!(
            secret = secret_name,
            namespace = namespace,
            "Access allowed: secret has XERV label"
        );

        AdmissionResponse {
            uid,
            allowed: true,
            status: None,
        }
    } else {
        warn!(
            secret = secret_name,
            namespace = namespace,
            labels = ?labels,
            "Access denied: secret missing required label"
        );

        AdmissionResponse {
            uid,
            allowed: false,
            status: Some(AdmissionStatus {
                code: 403,
                message: format!(
                    "Access denied: secret '{}' must have label 'app.kubernetes.io/managed-by=xerv' or 'app.kubernetes.io/managed-by=xerv-operator'",
                    secret_name
                ),
            }),
        }
    }
}

/// Safely build a response with a fallback if builder fails.
///
/// This should never fail in practice since we're building simple responses,
/// but we handle errors defensively to prevent webhook crashes.
fn build_response(status: StatusCode, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(body.into()))
        .unwrap_or_else(|e| {
            error!(error = %e, "Failed to build response - this should never happen");
            // Create an absolute minimal fallback response
            Response::new(Full::new(Bytes::from("Internal Error")))
        })
}

/// Health check endpoint
fn health_check() -> Response<Full<Bytes>> {
    build_response(StatusCode::OK, "OK")
}

/// Readiness check endpoint
fn ready_check() -> Response<Full<Bytes>> {
    build_response(StatusCode::OK, "Ready")
}

/// 404 response
fn not_found() -> Response<Full<Bytes>> {
    build_response(StatusCode::NOT_FOUND, "Not Found")
}

/// Error response
fn error_response(message: &str) -> Response<Full<Bytes>> {
    build_response(StatusCode::BAD_REQUEST, message.to_string())
}
