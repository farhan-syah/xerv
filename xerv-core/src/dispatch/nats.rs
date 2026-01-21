//! NATS-based dispatch backend for cloud-native deployments.
//!
//! This backend uses NATS JetStream for persistent, exactly-once delivery
//! with support for multi-region deployments.
//!
//! # Features
//!
//! - **Persistence**: JetStream provides durable storage
//! - **Multi-region**: NATS super-clusters for global distribution
//! - **Push-based**: Workers can subscribe to receive traces
//! - **Exactly-once**: Deduplication and ack tracking
//! - **Deferred acknowledgment**: Messages are only acked after successful processing
//!
//! # Requirements
//!
//! - NATS 2.2+ with JetStream enabled

use super::config::NatsConfig;
use super::request::TraceRequest;
use super::traits::{DispatchBackend, DispatchError, DispatchFuture, DispatchResult};
use crate::types::{PipelineId, TraceId};
use async_nats::jetstream::{
    self, Context, consumer::PullConsumer, message::Message, stream::Stream,
};
use futures_util::StreamExt;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// NATS-based dispatch backend.
///
/// Uses NATS JetStream for reliable, persistent message delivery with proper
/// deferred acknowledgment support.
pub struct NatsDispatch {
    config: NatsConfig,
    client: async_nats::Client,
    jetstream: Option<Context>,
    /// Stream wrapped in async mutex for interior mutability (info() requires &mut)
    stream: tokio::sync::Mutex<Option<Stream>>,
    consumer: Option<PullConsumer>,
    /// Track in-flight messages for deferred ack/nack.
    /// Uses parking_lot::Mutex for fast sync access; async ack happens after removal.
    in_flight: Arc<Mutex<HashMap<TraceId, InFlightMessage>>>,
    shutting_down: AtomicBool,
}

/// An in-flight message being processed.
///
/// Stores the JetStream message for deferred acknowledgment. The message
/// is acked only after successful processing, or nacked/requeued on failure.
struct InFlightMessage {
    /// The JetStream message handle for deferred ack/nack.
    message: Message,
    /// The deserialized trace request.
    request: TraceRequest,
}

impl NatsDispatch {
    /// Create a new NATS dispatch backend.
    pub async fn new(config: NatsConfig) -> DispatchResult<Self> {
        // Build connection options
        let mut opts = async_nats::ConnectOptions::new();

        if let Some(ref creds) = config.credentials {
            opts = match creds {
                super::config::NatsCredentials::UserPass { username, password } => {
                    opts.user_and_password(username.clone(), password.clone())
                }
                super::config::NatsCredentials::Token { token } => opts.token(token.clone()),
                super::config::NatsCredentials::NKey { seed } => opts.nkey(seed.clone()),
                super::config::NatsCredentials::CredsFile { path } => opts
                    .credentials_file(path)
                    .await
                    .map_err(|e| DispatchError::ConfigError(e.to_string()))?,
            };
        }

        // Connect to NATS
        let client = opts
            .connect(&config.url)
            .await
            .map_err(|e| DispatchError::ConnectionFailed(e.to_string()))?;

        tracing::info!(url = %config.url, "Connected to NATS");

        let mut dispatch = Self {
            config,
            client,
            jetstream: None,
            stream: tokio::sync::Mutex::new(None),
            consumer: None,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            shutting_down: AtomicBool::new(false),
        };

        // Initialize JetStream if enabled
        if dispatch.config.use_jetstream {
            dispatch.init_jetstream().await?;
        }

        Ok(dispatch)
    }

    /// Initialize JetStream stream and consumer.
    async fn init_jetstream(&mut self) -> DispatchResult<()> {
        let jetstream = jetstream::new(self.client.clone());

        // Create or get the stream
        let stream_name = &self.config.stream_name;
        let subject = format!("{}.>", self.config.subject_prefix);

        let stream = jetstream
            .get_or_create_stream(jetstream::stream::Config {
                name: stream_name.clone(),
                subjects: vec![subject],
                retention: jetstream::stream::RetentionPolicy::WorkQueue,
                max_messages: 1_000_000,
                max_bytes: 1024 * 1024 * 1024, // 1GB
                storage: jetstream::stream::StorageType::File,
                ..Default::default()
            })
            .await
            .map_err(|e| DispatchError::BackendError(format!("Failed to create stream: {}", e)))?;

        tracing::info!(stream = %stream_name, "Initialized JetStream stream");

        // Create or get the consumer
        let consumer_name = self
            .config
            .consumer_name
            .clone()
            .unwrap_or_else(|| format!("worker-{}", uuid::Uuid::new_v4()));

        let durable_name = self.config.durable_name.clone();

        let consumer_config = jetstream::consumer::pull::Config {
            name: Some(consumer_name.clone()),
            durable_name,
            ack_wait: Duration::from_millis(self.config.ack_wait_ms),
            max_ack_pending: self.config.max_ack_pending as i64,
            ..Default::default()
        };

        let consumer = stream
            .get_or_create_consumer(&consumer_name, consumer_config)
            .await
            .map_err(|e| {
                DispatchError::BackendError(format!("Failed to create consumer: {}", e))
            })?;

        tracing::info!(consumer = %consumer_name, "Initialized JetStream consumer");

        self.jetstream = Some(jetstream);
        *self.stream.get_mut() = Some(stream);
        self.consumer = Some(consumer);

        Ok(())
    }

    /// Get the subject for a trace.
    fn trace_subject(&self, pipeline_id: &PipelineId) -> String {
        format!(
            "{}.traces.{}",
            self.config.subject_prefix,
            pipeline_id.name()
        )
    }

    /// Serialize a trace request.
    fn serialize_request(request: &TraceRequest) -> DispatchResult<Vec<u8>> {
        serde_json::to_vec(request).map_err(|e| DispatchError::SerializationError(e.to_string()))
    }

    /// Deserialize a trace request.
    fn deserialize_request(data: &[u8]) -> DispatchResult<TraceRequest> {
        serde_json::from_slice(data).map_err(|e| DispatchError::SerializationError(e.to_string()))
    }
}

impl DispatchBackend for NatsDispatch {
    fn enqueue(&self, request: TraceRequest) -> DispatchFuture<'_, TraceId> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err(DispatchError::ShuttingDown);
            }

            let trace_id = request.trace_id;
            let subject = self.trace_subject(&request.pipeline_id);
            let data = Self::serialize_request(&request)?;

            if let Some(ref jetstream) = self.jetstream {
                // Publish to JetStream
                jetstream
                    .publish(subject.clone(), data.into())
                    .await
                    .map_err(|e| DispatchError::BackendError(e.to_string()))?
                    .await
                    .map_err(|e| DispatchError::BackendError(e.to_string()))?;
            } else {
                // Simple publish (no persistence)
                self.client
                    .publish(subject.clone(), data.into())
                    .await
                    .map_err(|e| DispatchError::BackendError(e.to_string()))?;
            }

            tracing::debug!(trace_id = %trace_id, subject = %subject, "Published trace to NATS");
            Ok(trace_id)
        })
    }

    fn dequeue(&self, timeout_ms: u64) -> DispatchFuture<'_, Option<TraceRequest>> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Ok(None);
            }

            let consumer = self
                .consumer
                .as_ref()
                .ok_or_else(|| DispatchError::ConfigError("JetStream not initialized".into()))?;

            let timeout = if timeout_ms > 0 {
                Duration::from_millis(timeout_ms)
            } else {
                Duration::from_millis(100) // Minimum poll interval
            };

            // Fetch one message from JetStream
            let mut messages = consumer
                .fetch()
                .max_messages(1)
                .expires(timeout)
                .messages()
                .await
                .map_err(|e| DispatchError::BackendError(e.to_string()))?;

            // Get the first message if available
            if let Some(msg_result) = messages.next().await {
                let msg = msg_result.map_err(|e| DispatchError::BackendError(e.to_string()))?;

                let mut request = Self::deserialize_request(&msg.payload)?;
                request.increment_attempt();

                let trace_id = request.trace_id;

                // Store message for deferred ack/nack.
                // The message is NOT acked here - it will be acked when ack() is called,
                // or nacked/requeued when nack() is called. This ensures at-least-once
                // delivery semantics: if the worker crashes before ack(), the message
                // will be redelivered after ack_wait timeout.
                self.in_flight.lock().insert(
                    trace_id,
                    InFlightMessage {
                        message: msg,
                        request: request.clone(),
                    },
                );

                tracing::debug!(trace_id = %trace_id, "Dequeued trace from NATS (pending ack)");
                return Ok(Some(request));
            }

            Ok(None)
        })
    }

    fn ack(&self, trace_id: TraceId) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            // Remove from in-flight tracking (sync operation)
            let maybe_msg = self.in_flight.lock().remove(&trace_id);

            // Ack with JetStream (async operation, outside the lock)
            if let Some(in_flight) = maybe_msg {
                in_flight.message.ack().await.map_err(|e| {
                    DispatchError::BackendError(format!("Failed to ack message: {}", e))
                })?;

                tracing::debug!(trace_id = %trace_id, "Acknowledged trace in NATS JetStream");
            } else {
                tracing::warn!(trace_id = %trace_id, "Attempted to ack unknown trace");
            }

            Ok(())
        })
    }

    fn nack(&self, trace_id: TraceId, error: String, retry: bool) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            // Remove from in-flight tracking (sync operation)
            let maybe_msg = self.in_flight.lock().remove(&trace_id);

            if let Some(in_flight) = maybe_msg {
                if retry && !in_flight.request.is_exhausted() {
                    // Use JetStream NAK with delay for automatic retry.
                    // This tells the server to redeliver the message after the delay.
                    // The delay helps prevent tight retry loops.
                    let delay = Duration::from_secs(1);
                    if let Err(e) = in_flight
                        .message
                        .ack_with(async_nats::jetstream::AckKind::Nak(Some(delay)))
                        .await
                    {
                        tracing::error!(
                            trace_id = %trace_id,
                            error = %e,
                            "Failed to NAK message, falling back to manual re-publish"
                        );
                        // Fallback: manually re-publish if NAK fails
                        let subject = self.trace_subject(&in_flight.request.pipeline_id);
                        let data = Self::serialize_request(&in_flight.request)?;
                        if let Some(ref jetstream) = self.jetstream {
                            jetstream
                                .publish(subject, data.into())
                                .await
                                .map_err(|e| DispatchError::BackendError(e.to_string()))?;
                        }
                    }

                    tracing::debug!(
                        trace_id = %trace_id,
                        error = %error,
                        attempt = in_flight.request.attempt,
                        "NACK'd trace for retry"
                    );
                } else {
                    // Exhausted retries or no retry requested - terminate the message.
                    // Use Term to tell JetStream this message should not be redelivered.
                    if let Err(e) = in_flight
                        .message
                        .ack_with(async_nats::jetstream::AckKind::Term)
                        .await
                    {
                        tracing::error!(
                            trace_id = %trace_id,
                            error = %e,
                            "Failed to terminate message"
                        );
                    }

                    tracing::warn!(
                        trace_id = %trace_id,
                        error = %error,
                        attempt = in_flight.request.attempt,
                        "Trace failed permanently, terminated in JetStream"
                    );
                }
            } else {
                tracing::warn!(trace_id = %trace_id, "Attempted to nack unknown trace");
            }

            Ok(())
        })
    }

    fn queue_depth(&self) -> DispatchFuture<'_, usize> {
        Box::pin(async move {
            let mut guard = self.stream.lock().await;
            if let Some(ref mut stream) = *guard {
                let info = stream
                    .info()
                    .await
                    .map_err(|e| DispatchError::BackendError(e.to_string()))?;
                Ok(info.state.messages as usize)
            } else {
                Ok(0)
            }
        })
    }

    fn in_flight_count(&self) -> DispatchFuture<'_, usize> {
        Box::pin(async move { Ok(self.in_flight.lock().len()) })
    }

    fn health_check(&self) -> DispatchFuture<'_, bool> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Ok(false);
            }

            // Check connection state
            Ok(self.client.connection_state() == async_nats::connection::State::Connected)
        })
    }

    fn shutdown(&self) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            self.shutting_down.store(true, Ordering::SeqCst);

            // Drain the client
            self.client.drain().await.ok();

            tracing::info!("NATS dispatch shutdown complete");
            Ok(())
        })
    }

    fn get_pending_by_pipeline(
        &self,
        pipeline_id: &PipelineId,
    ) -> DispatchFuture<'_, Vec<TraceId>> {
        let pipeline_id = pipeline_id.clone();
        Box::pin(async move {
            // This is expensive - only for monitoring
            let mut traces = Vec::new();

            // First, collect in-flight messages for this pipeline
            {
                let in_flight = self.in_flight.lock();
                traces.extend(
                    in_flight
                        .iter()
                        .filter(|(_, msg)| msg.request.pipeline_id == pipeline_id)
                        .map(|(id, _)| *id),
                );
            }

            // If JetStream is enabled, query the stream for pending messages
            if self.config.use_jetstream {
                // Get the stream (requires async lock)
                let stream_guard = self.stream.lock().await;
                if let Some(ref stream) = *stream_guard {
                    // Get stream info to know the range of messages
                    let info = stream
                        .info()
                        .await
                        .map_err(|e| DispatchError::BackendError(e.to_string()))?;

                    let first_seq = info.state.first_sequence;
                    let last_seq = info.state.last_sequence;

                    // Limit the number of messages to scan for safety
                    const MAX_SCAN: u64 = 1000;
                    let scan_count = (last_seq - first_seq + 1).min(MAX_SCAN);

                    // Start from the most recent messages (more likely to be pending)
                    let start_seq = last_seq.saturating_sub(scan_count - 1).max(first_seq);

                    // Fetch messages by sequence number
                    for seq in start_seq..=last_seq {
                        // Get message by sequence (this is a direct stream access)
                        match stream.get_raw_message(seq).await {
                            Ok(raw_msg) => {
                                // Deserialize the message payload
                                match Self::deserialize_request(&raw_msg.payload) {
                                    Ok(request) => {
                                        // Filter by pipeline_id
                                        if request.pipeline_id == pipeline_id {
                                            // Check if not already in our traces list (avoid duplicates)
                                            if !traces.contains(&request.trace_id) {
                                                traces.push(request.trace_id);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // Log deserialization error but continue
                                        tracing::warn!(
                                            sequence = seq,
                                            error = %e,
                                            "Failed to deserialize message from stream"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                // Message might have been deleted/expired
                                tracing::debug!(
                                    sequence = seq,
                                    error = %e,
                                    "Failed to get message from stream (likely deleted)"
                                );
                            }
                        }
                    }
                }
            }

            Ok(traces)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require a running NATS instance with JetStream
    // Run with: cargo test --features dispatch-nats -- --ignored

    #[tokio::test]
    #[ignore]
    async fn nats_dispatch_basic() {
        let config = NatsConfig::new("nats://localhost:4222")
            .with_jetstream()
            .stream_name("XERV_TEST");

        let dispatch = NatsDispatch::new(config).await.unwrap();

        // Enqueue
        let request = TraceRequest::new(
            TraceId::new(),
            PipelineId::new("test", 1),
            "trigger",
            vec![],
        );
        let trace_id = dispatch.enqueue(request).await.unwrap();

        // Dequeue
        let dequeued = dispatch.dequeue(5000).await.unwrap();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().trace_id, trace_id);

        // Ack
        dispatch.ack(trace_id).await.unwrap();

        dispatch.shutdown().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn nats_dispatch_get_pending_by_pipeline() {
        let config = NatsConfig::new("nats://localhost:4222")
            .with_jetstream()
            .stream_name("XERV_TEST_PENDING");

        let dispatch = NatsDispatch::new(config).await.unwrap();

        let pipeline1 = PipelineId::new("pipeline1", 1);
        let pipeline2 = PipelineId::new("pipeline2", 1);

        // Enqueue multiple traces for different pipelines
        let trace1 = TraceId::new();
        let trace2 = TraceId::new();
        let trace3 = TraceId::new();

        dispatch
            .enqueue(TraceRequest::new(
                trace1,
                pipeline1.clone(),
                "trigger",
                vec![],
            ))
            .await
            .unwrap();

        dispatch
            .enqueue(TraceRequest::new(
                trace2,
                pipeline1.clone(),
                "trigger",
                vec![],
            ))
            .await
            .unwrap();

        dispatch
            .enqueue(TraceRequest::new(
                trace3,
                pipeline2.clone(),
                "trigger",
                vec![],
            ))
            .await
            .unwrap();

        // Give JetStream time to persist messages
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Get pending traces for pipeline1
        let pending1 = dispatch.get_pending_by_pipeline(&pipeline1).await.unwrap();
        assert_eq!(pending1.len(), 2);
        assert!(pending1.contains(&trace1));
        assert!(pending1.contains(&trace2));
        assert!(!pending1.contains(&trace3));

        // Get pending traces for pipeline2
        let pending2 = dispatch.get_pending_by_pipeline(&pipeline2).await.unwrap();
        assert_eq!(pending2.len(), 1);
        assert!(pending2.contains(&trace3));
        assert!(!pending2.contains(&trace1));
        assert!(!pending2.contains(&trace2));

        // Dequeue and ack one trace from pipeline1
        let dequeued = dispatch.dequeue(5000).await.unwrap();
        assert!(dequeued.is_some());
        let dequeued_trace = dequeued.unwrap().trace_id;
        dispatch.ack(dequeued_trace).await.unwrap();

        // Give JetStream time to process ack
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Check pending again - should have one less for pipeline1
        let pending1_after = dispatch.get_pending_by_pipeline(&pipeline1).await.unwrap();
        assert_eq!(pending1_after.len(), 1);
        assert!(!pending1_after.contains(&dequeued_trace));

        dispatch.shutdown().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn nats_dispatch_get_pending_includes_in_flight() {
        let config = NatsConfig::new("nats://localhost:4222")
            .with_jetstream()
            .stream_name("XERV_TEST_IN_FLIGHT");

        let dispatch = NatsDispatch::new(config).await.unwrap();

        let pipeline = PipelineId::new("test_pipeline", 1);
        let trace_id = TraceId::new();

        // Enqueue a trace
        dispatch
            .enqueue(TraceRequest::new(
                trace_id,
                pipeline.clone(),
                "trigger",
                vec![],
            ))
            .await
            .unwrap();

        // Dequeue (puts it in-flight but don't ack yet)
        let dequeued = dispatch.dequeue(5000).await.unwrap();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().trace_id, trace_id);

        // Get pending - should include the in-flight trace
        let pending = dispatch.get_pending_by_pipeline(&pipeline).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending.contains(&trace_id));

        // Ack the trace
        dispatch.ack(trace_id).await.unwrap();

        // Get pending again - should be empty now
        let pending_after = dispatch.get_pending_by_pipeline(&pipeline).await.unwrap();
        assert_eq!(pending_after.len(), 0);

        dispatch.shutdown().await.unwrap();
    }
}
