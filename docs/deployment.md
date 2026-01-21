# XERV Deployment Guide

This guide covers deploying XERV in different environments, from local development to production cloud deployments.

## Quick Start by Scenario

### Local Development

```bash
cd docker
docker compose up -d
# API available at http://localhost:8080
```

See: [Docker Compose Guide](../docker/README.md)

### Single-Node Production (On-Premises)

```bash
# 1. Build binary
cargo build --release --bin xerv-cli

# 2. Deploy flow
./target/release/xerv-cli validate flows/order_processing.yaml

# 3. Start server
XERV_DISPATCH_BACKEND=raft \
XERV_API_PORT=8080 \
./target/release/xerv-cli serve
```

### High-Availability Cluster (On-Premises)

```bash
# 1. Use Helm chart with Raft backend
helm install xerv ./charts/xerv \
  --set dispatch.backend=raft \
  --set replicaCount=3
```

See: [Helm Chart Documentation](../charts/xerv/README.md)

### Cloud Deployment (AWS, GCP, Azure)

```bash
# 1. Use Helm chart with Redis or NATS backend
helm install xerv ./charts/xerv \
  --set dispatch.backend=redis \
  --set dispatch.redis.url=redis://redis-managed:6379 \
  --set replicaCount=5 \
  --set autoscaling.enabled=true
```

See: [Helm Chart Documentation](../charts/xerv/README.md#high-throughput-with-redis)

## Deployment Matrix

| Scenario             | Backend | Deployment            | Complexity | HA  | Scaling      |
| -------------------- | ------- | --------------------- | ---------- | --- | ------------ |
| **Dev/Testing**      | Memory  | Docker Compose        | Simple     | ❌  | Vertical     |
| **Single-node Prod** | Raft    | Binary or Docker      | Low        | ❌  | Vertical     |
| **On-Premises HA**   | Raft    | Helm/K8s              | Medium     | ✅  | Horizontal\* |
| **Cloud Standard**   | Redis   | Helm/K8s              | Medium     | ✅  | Horizontal   |
| **Cloud Serverless** | NATS    | Helm/K8s + KEDA       | High       | ✅  | Auto         |
| **Multi-Region**     | NATS    | Helm/K8s + Federation | High       | ✅  | Auto         |

\*Raft requires careful management of cluster membership (add/remove one node at a time, maintain odd quorum)

## Backend Selection Guide

### Memory Backend

**Use for:** Development, testing, single-machine edge deployments

**Deployment:**

```bash
docker compose up -d
# or
XERV_DISPATCH_BACKEND=memory ./target/release/xerv-cli serve
```

**Characteristics:**

- Zero external dependencies
- Maximum performance (10k+ traces/sec)
- Single point of failure
- All data in-memory (lost on restart)

### Raft Backend

**Use for:** Production on-premises, high availability, zero external dependencies required

**Deployment (3-node cluster):**

```yaml
# values-raft.yaml
replicaCount: 3
dispatch:
  backend: raft
  raft:
    electionTimeoutMs: 1000
    heartbeatIntervalMs: 100
storage:
  class: fast-ssd
  size: 50Gi
podDisruptionBudget:
  minAvailable: 2
```

```bash
helm install xerv ./charts/xerv -f values-raft.yaml
```

**Characteristics:**

- No external dependencies
- Automatic leader election and failover
- Odd number of replicas required (3, 5, 7)
- Careful scaling (add/remove one node at a time)
- gRPC peer communication

**Scaling Raft Cluster:**

Add new node:

```bash
# 1. Add replica to StatefulSet
kubectl scale statefulset xerv --replicas=5

# 2. Configure new node to contact existing peers
# 3. New node starts as learner, becomes voter after log catch-up

# Monitor leader
kubectl logs xerv-0 | grep raft
```

Remove node:

```bash
# 1. Remove replica
kubectl scale statefulset xerv --replicas=3

# 2. Existing cluster automatically removes from membership
```

**Maintenance:**

- Monitor Raft leader status: `/health` endpoint shows leader info
- Keep backups of persistent volumes
- Plan for leader failover (1-2 second pause)

### Redis Backend

**Use for:** Cloud deployments, high throughput needed, don't want operational burden of Raft

**Deployment (10 workers):**

```yaml
# values-redis.yaml
replicaCount: 10
dispatch:
  backend: redis
  redis:
    url: redis://redis-managed.example.com:6379
    useStreams: true
    poolSize: 20
autoscaling:
  enabled: true
  minReplicas: 5
  maxReplicas: 20
  targetCPUUtilizationPercentage: 80
```

```bash
helm install xerv ./charts/xerv -f values-redis.yaml
```

**Characteristics:**

- Stateless workers (any pod can process any trace)
- Horizontal scaling without coordination
- External Redis dependency (managed service recommended)
- 50k+ traces/sec throughput
- Automatic consumer group load balancing

**Scaling:**

```bash
# Scale up
kubectl scale deployment xerv --replicas=20

# Scale down (pods drain gracefully)
kubectl scale deployment xerv --replicas=10
```

**High Availability:**

- Use Redis Cluster for high availability
- Connection pooling handles pod failures
- Traces not lost (persisted in Redis Streams)

**Redis Setup (Managed Service):**

- AWS ElastiCache with cluster mode
- Google Cloud Memorystore
- Azure Cache for Redis Enterprise

### NATS Backend

**Use for:** Streaming architecture, multi-region deployments, serverless/auto-scale

**Deployment (5 workers, auto-scaling):**

```yaml
# values-nats.yaml
replicaCount: 5
dispatch:
  backend: nats
  nats:
    url: nats://nats-cluster:4222
    useJetstream: true
keda:
  enabled: true
  minReplicaCount: 1 # Scale to 1
  maxReplicaCount: 50
  triggers:
    pendingTraces:
      enabled: true
      threshold: "50"
```

```bash
helm install xerv ./charts/xerv -f values-nats.yaml
```

**Characteristics:**

- Stateless workers (auto-scale friendly)
- NATS JetStream for persistence
- Multi-region super-clusters supported
- 50k+ traces/sec throughput
- KEDA integration for scale-to-zero

**Scaling with KEDA:**

```bash
# Install KEDA
helm repo add kedacore https://kedacore.github.io/charts
helm install keda kedacore/keda --namespace keda --create-namespace

# Install XERV with KEDA enabled
helm install xerv ./charts/xerv -f values-nats.yaml

# Monitor scaling decisions
kubectl get scaledobjects -n xerv
kubectl logs -n keda deployment/keda-operator
```

**Multi-Region Setup:**

```yaml
# NATS super-cluster
spec:
  nats:
    url: "nats://cluster1:4222,nats://cluster2:4222,nats://cluster3:4222"
```

## Environment Variables

### Common

| Variable                | Default     | Description                                |
| ----------------------- | ----------- | ------------------------------------------ |
| `XERV_DISPATCH_BACKEND` | `memory`    | Backend: `memory`, `raft`, `redis`, `nats` |
| `XERV_API_PORT`         | `8080`      | HTTP API port                              |
| `XERV_DATA_DIR`         | `/tmp/xerv` | Data directory for arena/WAL               |
| `XERV_METRICS_ENABLED`  | `true`      | Enable Prometheus metrics                  |
| `RUST_LOG`              | `xerv=info` | Log level                                  |

### Dispatch-Specific

**Raft:**

```bash
XERV_DISPATCH_RAFT_NODE_ID=1
XERV_DISPATCH_RAFT_LISTEN_ADDR="0.0.0.0:5000"
XERV_DISPATCH_RAFT_PEERS="2:node2:5000,3:node3:5000"
```

**Redis:**

```bash
XERV_DISPATCH_REDIS_URL="redis://redis:6379"
XERV_DISPATCH_REDIS_POOL_SIZE=20
XERV_DISPATCH_REDIS_USE_STREAMS=true
```

**NATS:**

```bash
XERV_DISPATCH_NATS_URL="nats://nats:4222"
XERV_DISPATCH_NATS_USE_JETSTREAM=true
```

### Observability

```bash
# Metrics
XERV_METRICS_PORT=9090

# Logging
XERV_LOG_FORMAT="json"  # json, pretty, or compact
RUST_LOG="xerv=debug"

# Tracing
OTEL_ENABLED=true
OTEL_EXPORTER_OTLP_ENDPOINT="http://jaeger-collector:4317"
OTEL_SERVICE_NAME="xerv-production"
```

## Production Checklist

### Before Deploying

- [ ] Choose dispatch backend (Memory for edge, Raft for on-prem, Redis/NATS for cloud)
- [ ] Set up persistent storage (if using Raft)
- [ ] Configure backups (persistent volumes, Redis snapshots, NATS snapshots)
- [ ] Set up monitoring (Prometheus metrics, Grafana dashboards)
- [ ] Configure alerting (PrometheusRule, PagerDuty, Slack)
- [ ] Test failover scenarios
- [ ] Document run-books for common operations

### Deployment Day

```bash
# 1. Validate configuration
helm template xerv ./charts/xerv -f values-prod.yaml | kubectl apply -f - --dry-run=client

# 2. Deploy
helm install xerv ./charts/xerv -f values-prod.yaml --wait

# 3. Verify health
kubectl get pods -l app.kubernetes.io/name=xerv
curl http://localhost:8080/health

# 4. Test trace execution
curl -X POST http://localhost:8080/api/traces/execute \
  -H "Content-Type: application/json" \
  -d '{...}'
```

### Post-Deployment

- [ ] Verify all pods are running
- [ ] Check metrics are flowing to Prometheus
- [ ] Test alerting (trigger a test alert)
- [ ] Monitor error rates for first 24 hours
- [ ] Document actual behavior vs. capacity planning

## Troubleshooting

### Raft Cluster Issues

**Problem:** "No raft leader"

```bash
# Check pod logs
kubectl logs xerv-0 | grep raft

# Verify replica count is odd
kubectl get statefulset xerv -o jsonpath='{.spec.replicas}'

# Verify network connectivity between pods
kubectl exec -it xerv-0 -- nslookup xerv-1.xerv
```

**Problem:** "Pod stuck in pending"

```bash
# Check PVC status
kubectl get pvc

# Check node capacity
kubectl top nodes

# Check events
kubectl describe pod xerv-0
```

### Redis Backend Issues

**Problem:** "Cannot connect to Redis"

```bash
# Verify Redis URL
kubectl get configmap xerv -o jsonpath='{.data.XERV_DISPATCH_REDIS_URL}'

# Test connectivity from pod
kubectl exec -it xerv-0 -- redis-cli -u "$XERV_DISPATCH_REDIS_URL" ping

# Check connection pool
kubectl logs xerv-0 | grep redis
```

**Problem:** "Queue depth growing"

```bash
# Check active worker count
kubectl get pods -l app.kubernetes.io/name=xerv

# Scale up
kubectl scale deployment xerv --replicas=20

# Monitor metrics
kubectl port-forward svc/xerv 9090:9090
# Visit http://localhost:9090/graph and query xerv_dispatch_queue_depth
```

### General Issues

**Problem:** "Traces not processing"

```bash
# Check if pipeline is paused
curl http://localhost:8080/health
curl http://localhost:8080/pipelines

# Check for suspended traces
curl http://localhost:8080/suspensions

# View logs
kubectl logs -l app.kubernetes.io/name=xerv --all-containers=true -f
```

**Problem:** "High latency"

```bash
# Check node execution metrics
kubectl port-forward svc/xerv 9090:9090
# Query: histogram_quantile(0.95, xerv_node_execution_seconds_bucket)

# Check if nodes are CPU-bound
kubectl top pods -l app.kubernetes.io/name=xerv

# Check if network-bound (Redis/NATS latency)
kubectl exec -it xerv-0 -- redis-cli --latency
```

## Backup & Disaster Recovery

### Raft Backend

**Backing up state:**

```bash
# Backup persistent volumes
kubectl get pvc -l app.kubernetes.io/name=xerv
# Back up underlying storage (EBS, GCE Persistent Disk, etc.)
```

**Recovery:**

```bash
# Restore PVC from backup
# Restart pod
kubectl delete pod xerv-0
# Pod will recover state from persistent volume
```

### Redis Backend

**Backing up:**

```bash
# Enable Redis persistence (snapshots)
# Managed service handles this automatically

# Manual backup
redis-cli --uri redis://redis-cluster:6379 BGSAVE
```

**Recovery:**

```bash
# Redis automatically replays from snapshots
# Redeployed workers pick up from Redis consumer group
```

### NATS Backend

**Backing up:**

```bash
# JetStream persists to filesystem or S3
# Managed service handles this automatically
```

**Recovery:**

```bash
# NATS redelivers unacked messages
# Redeployed workers process from where they left off
```

## Performance Tuning

### Memory Backend (Local)

- CPU bottleneck: Add async task workers (tokio)
- Memory bottleneck: Increase available memory
- Peak: 10k+ traces/sec on modern hardware

### Raft Backend

- CPU bottleneck: Upgrade node resources
- Consensus bottleneck: Reduce log replication frequency
- Network bottleneck: Use dedicated network or gRPC compression
- Typical: Limited by Raft consensus, ~1k traces/sec

### Redis Backend

- CPU bottleneck: Increase worker count
- Network bottleneck: Redis connection pooling (deadpool)
- Redis bottleneck: Use Redis Cluster or managed service
- Peak: 50k+ traces/sec with sufficient workers

### NATS Backend

- Similar to Redis
- Additional tuning: JetStream batch size, pull interval
- Peak: 50k+ traces/sec with sufficient workers

## See Also

- **[Helm Chart Documentation](../charts/xerv/README.md)** - Complete Kubernetes configuration
- **[Docker Compose Guide](../docker/README.md)** - Local development setup
- **[REST API Reference](api.md)** - Management endpoints
- **[Architecture](architecture.md)** - Understanding dispatch backends
