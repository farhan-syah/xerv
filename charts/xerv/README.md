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
