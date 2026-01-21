# XERV Helm Chart

Helm chart for deploying XERV workflow orchestration platform on Kubernetes.

## Prerequisites

- Kubernetes 1.25+
- Helm 3.0+
- PV provisioner (for Raft backend)

## Installation

```bash
# Add the repository (when published)
helm repo add xerv https://charts.xerv.io
helm repo update

# Install with default values (memory backend)
helm install xerv xerv/xerv

# Install with Raft backend (3 replicas)
helm install xerv xerv/xerv \
  --set dispatch.backend=raft \
  --set replicaCount=3

# Install with Redis backend
helm install xerv xerv/xerv \
  --set dispatch.backend=redis \
  --set dispatch.redis.url=redis://redis:6379

# Install with NATS backend
helm install xerv xerv/xerv \
  --set dispatch.backend=nats \
  --set dispatch.nats.url=nats://nats:4222
```

## Configuration

### Common Parameters

| Parameter          | Description        | Default                      |
| ------------------ | ------------------ | ---------------------------- |
| `replicaCount`     | Number of replicas | `1`                          |
| `image.repository` | Image repository   | `ghcr.io/ml-rust/xerv`       |
| `image.tag`        | Image tag          | `""` (uses Chart.appVersion) |
| `image.pullPolicy` | Image pull policy  | `IfNotPresent`               |

### Dispatch Backend

| Parameter                    | Description                                     | Default  |
| ---------------------------- | ----------------------------------------------- | -------- |
| `dispatch.backend`           | Backend type: `memory`, `raft`, `redis`, `nats` | `memory` |
| `dispatch.redis.url`         | Redis connection URL                            | `""`     |
| `dispatch.redis.useStreams`  | Use Redis Streams                               | `true`   |
| `dispatch.nats.url`          | NATS connection URL                             | `""`     |
| `dispatch.nats.useJetstream` | Use JetStream                                   | `true`   |

### Server

| Parameter         | Description      | Default |
| ----------------- | ---------------- | ------- |
| `server.apiPort`  | HTTP API port    | `8080`  |
| `server.grpcPort` | gRPC port (Raft) | `5000`  |

### Concurrency & Performance

| Parameter                      | Description                           | Default |
| ------------------------------ | ------------------------------------- | ------- |
| `executor.maxConcurrentNodes`  | Max concurrent nodes per trace        | `16`    |
| `executor.maxConcurrentTraces` | Max concurrent traces                 | `100`   |
| `executor.nodeTimeoutMs`       | Node execution timeout (milliseconds) | `30000` |

### Storage (Raft backend)

| Parameter            | Description   | Default         |
| -------------------- | ------------- | --------------- |
| `storage.class`      | Storage class | `""` (default)  |
| `storage.size`       | Storage size  | `10Gi`          |
| `storage.accessMode` | Access mode   | `ReadWriteOnce` |

### Resources

| Parameter                   | Description    | Default |
| --------------------------- | -------------- | ------- |
| `resources.requests.cpu`    | CPU request    | `100m`  |
| `resources.requests.memory` | Memory request | `256Mi` |
| `resources.limits.cpu`      | CPU limit      | `1000m` |
| `resources.limits.memory`   | Memory limit   | `1Gi`   |

### Metrics

| Parameter                        | Description           | Default |
| -------------------------------- | --------------------- | ------- |
| `metrics.enabled`                | Enable metrics        | `true`  |
| `metrics.port`                   | Metrics port          | `9090`  |
| `metrics.serviceMonitor.enabled` | Enable ServiceMonitor | `false` |

### Ingress

| Parameter           | Description       | Default |
| ------------------- | ----------------- | ------- |
| `ingress.enabled`   | Enable ingress    | `false` |
| `ingress.className` | Ingress class     | `""`    |
| `ingress.hosts`     | Ingress hosts     | `[]`    |
| `ingress.tls`       | TLS configuration | `[]`    |

### Service Mesh

| Parameter                                | Description                    | Default    |
| ---------------------------------------- | ------------------------------ | ---------- |
| `serviceMesh.enabled`                    | Enable service mesh            | `false`    |
| `serviceMesh.provider`                   | Provider: `istio` or `linkerd` | `""`       |
| `serviceMesh.istio.injection.enabled`    | Enable Istio sidecar injection | `true`     |
| `serviceMesh.istio.destinationRule.*`    | DestinationRule config         | See values |
| `serviceMesh.istio.virtualService.*`     | VirtualService config          | See values |
| `serviceMesh.istio.peerAuthentication.*` | mTLS configuration             | See values |
| `serviceMesh.linkerd.injection.enabled`  | Enable Linkerd injection       | `true`     |
| `serviceMesh.linkerd.serviceProfile.*`   | ServiceProfile config          | See values |
| `serviceMesh.linkerd.trafficSplit.*`     | TrafficSplit for canary        | See values |

### Network Policy

| Parameter               | Description              | Default |
| ----------------------- | ------------------------ | ------- |
| `networkPolicy.enabled` | Enable NetworkPolicy     | `false` |
| `networkPolicy.ingress` | Additional ingress rules | `[]`    |
| `networkPolicy.egress`  | Additional egress rules  | `[]`    |

### KEDA (Kubernetes Event-Driven Autoscaling)

| Parameter                               | Description                            | Default |
| --------------------------------------- | -------------------------------------- | ------- |
| `keda.enabled`                          | Enable KEDA ScaledObject               | `false` |
| `keda.minReplicaCount`                  | Minimum replicas (0 for scale-to-zero) | `0`     |
| `keda.maxReplicaCount`                  | Maximum replicas                       | `10`    |
| `keda.cooldownPeriod`                   | Cooldown before scale-down (seconds)   | `300`   |
| `keda.pollingInterval`                  | Metrics polling interval (seconds)     | `30`    |
| `keda.triggers.pendingTraces.enabled`   | Scale on pending traces                | `false` |
| `keda.triggers.pendingTraces.threshold` | Pending traces threshold               | `"50"`  |
| `keda.triggers.activeTraces.enabled`    | Scale on active traces                 | `false` |
| `keda.triggers.activeTraces.threshold`  | Active traces threshold                | `"100"` |

### Vertical Pod Autoscaler (VPA)

| Parameter               | Description                           | Default    |
| ----------------------- | ------------------------------------- | ---------- |
| `vpa.enabled`           | Enable VPA                            | `false`    |
| `vpa.updateMode`        | Update mode: `Off`, `Initial`, `Auto` | `"Auto"`   |
| `vpa.containerPolicies` | Container resource policies           | See values |

## Examples

### Production Raft Cluster

```yaml
# values-production.yaml
replicaCount: 5

dispatch:
  backend: raft
  raft:
    electionTimeoutMs: 1000
    heartbeatIntervalMs: 100

storage:
  class: fast-ssd
  size: 50Gi

resources:
  requests:
    cpu: 500m
    memory: 1Gi
  limits:
    cpu: 2000m
    memory: 4Gi

podDisruptionBudget:
  enabled: true
  minAvailable: 3

affinity:
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      - labelSelector:
          matchLabels:
            app.kubernetes.io/name: xerv
        topologyKey: kubernetes.io/hostname
```

### High-Throughput with Redis

```yaml
# values-redis.yaml
replicaCount: 10

dispatch:
  backend: redis
  redis:
    url: redis://redis-cluster:6379
    useStreams: true
    poolSize: 20

resources:
  requests:
    cpu: 200m
    memory: 512Mi
  limits:
    cpu: 1000m
    memory: 2Gi
```

### Istio Service Mesh

```yaml
# values-istio.yaml
serviceMesh:
  enabled: true
  provider: istio
  istio:
    injection:
      enabled: true
    destinationRule:
      enabled: true
      trafficPolicy:
        connectionPool:
          http:
            h2UpgradePolicy: UPGRADE
            http2MaxRequests: 1000
        loadBalancer:
          simple: ROUND_ROBIN
          localityLbSetting:
            enabled: true
        outlierDetection:
          consecutive5xxErrors: 5
          interval: 30s
          baseEjectionTime: 30s
      tlsMode: ISTIO_MUTUAL
    virtualService:
      enabled: true
      timeout: 30s
      retries:
        attempts: 3
        perTryTimeout: 10s
    peerAuthentication:
      enabled: true
      mtls: STRICT
```

### Linkerd Service Mesh

```yaml
# values-linkerd.yaml
serviceMesh:
  enabled: true
  provider: linkerd
  linkerd:
    injection:
      enabled: true
    serviceProfile:
      enabled: true
      retryBudget:
        retryRatio: 0.2
        minRetriesPerSecond: 10
        ttl: 10s
    # Enable for canary deployments
    trafficSplit:
      enabled: false
```

### Network Policy

```yaml
# values-network-secure.yaml
networkPolicy:
  enabled: true
  # Allow traffic from a specific namespace
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              name: frontend
      ports:
        - protocol: TCP
          port: 8080
  # Allow egress to external API
  egress:
    - to:
        - ipBlock:
            cidr: 10.0.0.0/8
      ports:
        - protocol: TCP
          port: 443
```

### KEDA Autoscaling (Scale-to-Zero)

```yaml
# values-keda.yaml
# Disable HPA when using KEDA
autoscaling:
  enabled: false

keda:
  enabled: true
  minReplicaCount: 0 # Scale to zero when idle
  maxReplicaCount: 20
  cooldownPeriod: 300
  pollingInterval: 15
  fallback:
    enabled: true
    replicas: 2
  triggers:
    pendingTraces:
      enabled: true
      serverAddress: http://prometheus-server.monitoring.svc.cluster.local
      threshold: "50"
    activeTraces:
      enabled: true
      serverAddress: http://prometheus-server.monitoring.svc.cluster.local
      threshold: "100"
```

### Vertical Pod Autoscaler

```yaml
# values-vpa.yaml
vpa:
  enabled: true
  updateMode: "Auto"
  containerPolicies:
    - containerName: "*"
      minAllowed:
        cpu: 100m
        memory: 256Mi
      maxAllowed:
        cpu: 4
        memory: 8Gi
      controlledResources:
        - cpu
        - memory
      controlledValues: "RequestsAndLimits"
```

## Service Mesh Integration

### Istio Setup

1. **Prerequisites**: Install Istio with `istioctl install --set profile=default`

2. **Enable namespace injection** (optional, chart handles pod annotations):

   ```bash
   kubectl label namespace xerv istio-injection=enabled
   ```

3. **Install XERV with Istio**:

   ```bash
   helm install xerv xerv/xerv -f values-istio.yaml
   ```

4. **Verify sidecar injection**:
   ```bash
   kubectl get pods -n xerv -o jsonpath='{.items[*].spec.containers[*].name}'
   # Should show: xerv istio-proxy
   ```

### Linkerd Setup

1. **Prerequisites**: Install Linkerd with `linkerd install | kubectl apply -f -`

2. **Install XERV with Linkerd**:

   ```bash
   helm install xerv xerv/xerv -f values-linkerd.yaml
   ```

3. **Verify proxy injection**:
   ```bash
   kubectl get pods -n xerv -o jsonpath='{.items[*].spec.containers[*].name}'
   # Should show: xerv linkerd-proxy
   ```

### Multi-Cluster Mesh Setup

For multi-region federation with service mesh, you need to configure cross-cluster communication.

#### Istio Multi-Cluster

1. **Configure trust** between clusters (shared root CA or external CA):

   ```bash
   # On each cluster
   istioctl install --set values.global.meshID=mesh1 \
     --set values.global.multiCluster.clusterName=cluster1 \
     --set values.global.network=network1
   ```

2. **Enable cross-cluster service discovery**:

   ```bash
   # Create remote secret for each cluster
   istioctl create-remote-secret --name=cluster2 | \
     kubectl apply -f - --context=cluster1
   ```

3. **Deploy XERV on each cluster** with federation CRDs:
   ```yaml
   # Create XervFederation resource (see xerv-operator docs)
   apiVersion: xerv.io/v1
   kind: XervFederation
   metadata:
     name: global
   spec:
     clusters:
       - name: us-east
         endpoint: https://xerv.us-east.example.com
       - name: eu-west
         endpoint: https://xerv.eu-west.example.com
     routing:
       defaultStrategy: nearest
     security:
       mtlsEnabled: true
   ```

#### Linkerd Multi-Cluster

1. **Install multi-cluster extension**:

   ```bash
   linkerd multicluster install | kubectl apply -f -
   ```

2. **Link clusters**:

   ```bash
   # On target cluster, get credentials
   linkerd multicluster link --cluster-name=cluster2 | \
     kubectl apply -f - --context=cluster1
   ```

3. **Export XERV service**:

   ```bash
   kubectl label svc xerv mirror.linkerd.io/exported=true
   ```

4. **Deploy XERV with federation** using the same XervFederation CRD.

### gRPC with Service Mesh

When using Raft backend with service mesh, gRPC traffic between peers is automatically configured:

- **Istio**: Uses `ISTIO_MUTUAL` TLS by default
- **Linkerd**: Automatically handles HTTP/2 (gRPC) traffic

For optimal performance, the chart creates separate DestinationRule/ServiceProfile for the headless service used by Raft peer communication.

## Observability

XERV provides comprehensive observability through metrics, logging, and distributed tracing.

### Prometheus Metrics

XERV exposes Prometheus metrics at `/metrics` (port 9090 by default).

**Enable ServiceMonitor for Prometheus Operator:**

```yaml
metrics:
  enabled: true
  serviceMonitor:
    enabled: true
    interval: 30s
```

**Key Metrics:**

| Metric                        | Type      | Description                |
| ----------------------------- | --------- | -------------------------- |
| `xerv_completed_traces_total` | Counter   | Total completed traces     |
| `xerv_failed_traces_total`    | Counter   | Total failed traces        |
| `xerv_nodes_executed_total`   | Counter   | Total nodes executed       |
| `xerv_active_traces`          | Gauge     | Currently executing traces |
| `xerv_pending_traces`         | Gauge     | Traces waiting to execute  |
| `xerv_dispatch_queue_depth`   | Gauge     | Dispatch queue depth       |
| `xerv_raft_leader`            | Gauge     | Raft leader status (1/0)   |
| `xerv_trace_duration_seconds` | Histogram | Trace execution duration   |
| `xerv_node_execution_seconds` | Histogram | Node execution duration    |
| `xerv_arena_size_bytes`       | Gauge     | Arena memory usage         |

### Grafana Dashboards

Enable Grafana dashboard provisioning:

```yaml
monitoring:
  dashboards:
    enabled: true
    labels:
      grafana_dashboard: "1"
```

This creates a ConfigMap with the XERV Overview dashboard that Grafana can auto-discover.

### Prometheus Alerts

Enable PrometheusRule for alerting:

```yaml
monitoring:
  alerts:
    enabled: true
    # Customize thresholds
    errorRateThreshold: 0.05 # 5% error rate warning
    queueDepthWarning: 100 # Queue depth warning
    queueDepthCritical: 500 # Queue depth critical
    arenaSizeWarning: 1073741824 # 1GB arena warning
```

**Included Alerts:**

- `XervHighErrorRate` - Error rate exceeds threshold
- `XervCriticalErrorRate` - Error rate critically high
- `XervQueueDepthHigh` - Dispatch queue backing up
- `XervRaftLeaderMissing` - No Raft leader (Raft backend only)
- `XervRaftLeaderFlapping` - Raft leader changes frequently
- `XervArenaStorageHigh` - Arena memory usage high
- `XervSlowTraceExecution` - Traces executing slowly
- `XervNoTracesProcessed` - Traces stuck in queue
- `XervInstanceDown` - Instance not responding

### Logging with Loki/ELK

XERV supports structured JSON logging for easy integration with Loki, ELK, or other log aggregators.

**Configure JSON logging:**

```yaml
monitoring:
  logging:
    format: json # json, pretty, or compact
    level: info # trace, debug, info, warn, error

extraEnv:
  - name: XERV_LOG_FORMAT
    value: json
  - name: RUST_LOG
    value: xerv=info
```

**Loki Integration (via Promtail):**

```yaml
# promtail-config.yaml
scrape_configs:
  - job_name: xerv
    kubernetes_sd_configs:
      - role: pod
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app_kubernetes_io_name]
        regex: xerv
        action: keep
    pipeline_stages:
      - json:
          expressions:
            level: level
            trace_id: trace_id
            node_id: node_id
            pipeline_id: pipeline_id
            message: message
      - labels:
          level:
          trace_id:
          pipeline_id:
```

**ELK Integration (via Filebeat):**

```yaml
# filebeat.yml
filebeat.autodiscover:
  providers:
    - type: kubernetes
      templates:
        - condition:
            equals:
              kubernetes.labels.app.kubernetes.io/name: xerv
          config:
            - type: container
              paths:
                - /var/log/containers/*-${data.kubernetes.container.id}.log
              processors:
                - decode_json_fields:
                    fields: ["message"]
                    target: "xerv"
                    overwrite_keys: true
```

**Example JSON log output:**

```json
{
  "timestamp": "2024-01-15T10:30:45.123Z",
  "level": "INFO",
  "target": "xerv_executor::scheduler",
  "trace_id": "550e8400-e29b-41d4-a716-446655440000",
  "node_id": 5,
  "pipeline_id": "order-processing",
  "message": "Node completed",
  "fields": {
    "duration_ms": 42,
    "output_port": "out"
  }
}
```

### Distributed Tracing with Jaeger/Tempo

XERV supports OpenTelemetry for distributed tracing, compatible with Jaeger, Tempo, and other OTLP-compatible backends.

**Enable OpenTelemetry tracing:**

```yaml
monitoring:
  tracing:
    enabled: true
    endpoint: "http://jaeger-collector:4317"
    serviceName: "xerv"

extraEnv:
  - name: OTEL_ENABLED
    value: "true"
  - name: OTEL_EXPORTER_OTLP_ENDPOINT
    value: "http://jaeger-collector:4317"
  - name: OTEL_SERVICE_NAME
    value: "xerv"
```

**Jaeger Deployment:**

```bash
# Install Jaeger operator
kubectl create namespace observability
kubectl apply -f https://github.com/jaegertracing/jaeger-operator/releases/download/v1.50.0/jaeger-operator.yaml -n observability

# Create Jaeger instance
kubectl apply -f - <<EOF
apiVersion: jaegertracing.io/v1
kind: Jaeger
metadata:
  name: jaeger
  namespace: observability
spec:
  strategy: production
  storage:
    type: elasticsearch
    options:
      es:
        server-urls: http://elasticsearch:9200
EOF
```

**Tempo Deployment (with Grafana):**

```bash
# Install Tempo via Helm
helm repo add grafana https://grafana.github.io/helm-charts
helm install tempo grafana/tempo \
  --set tempo.receivers.otlp.protocols.grpc.endpoint=0.0.0.0:4317
```

**Trace Spans:**

XERV creates the following spans:

- `trace_execution` - Root span for entire trace execution
- `node_execution` - Child span for each node
- `suspend_trace` - Span for trace suspension
- `resume_trace` - Span for trace resumption

Each span includes:

- `trace_id` - XERV trace identifier
- `node_id` - Node being executed
- `pipeline_id` - Pipeline identifier
- `node_type` - Node type (e.g., `std::http`)

### Complete Observability Stack Example

Deploy XERV with full observability:

```yaml
# values-observability.yaml
metrics:
  enabled: true
  serviceMonitor:
    enabled: true
    interval: 15s

monitoring:
  alerts:
    enabled: true
    errorRateThreshold: 0.05
    queueDepthCritical: 500
  dashboards:
    enabled: true
    labels:
      grafana_dashboard: "1"
  logging:
    format: json
    level: info
  tracing:
    enabled: true
    endpoint: "http://tempo:4317"
    serviceName: "xerv-production"

extraEnv:
  - name: XERV_LOG_FORMAT
    value: json
  - name: OTEL_ENABLED
    value: "true"
  - name: OTEL_EXPORTER_OTLP_ENDPOINT
    value: "http://tempo:4317"
```

```bash
helm install xerv xerv/xerv -f values-observability.yaml
```

## Upgrading

```bash
helm upgrade xerv xerv/xerv -f values.yaml
```

## Uninstalling

```bash
helm uninstall xerv
```

**Note:** PVCs are not automatically deleted. To remove data:

```bash
kubectl delete pvc -l app.kubernetes.io/name=xerv
```
