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

### Service Mesh

| Parameter                              | Description                    | Default        |
| -------------------------------------- | ------------------------------ | -------------- |
| `serviceMesh.enabled`                  | Enable service mesh            | `false`        |
| `serviceMesh.provider`                 | Provider: `istio` or `linkerd` | `""`           |
| `serviceMesh.istio.injection.enabled`  | Enable Istio sidecar injection | `true`         |
| `serviceMesh.istio.destinationRule.*`  | DestinationRule config         | See values     |
| `serviceMesh.istio.virtualService.*`   | VirtualService config          | See values     |
| `serviceMesh.istio.peerAuthentication.*` | mTLS configuration           | See values     |
| `serviceMesh.linkerd.injection.enabled`| Enable Linkerd injection       | `true`         |
| `serviceMesh.linkerd.serviceProfile.*` | ServiceProfile config          | See values     |
| `serviceMesh.linkerd.trafficSplit.*`   | TrafficSplit for canary        | See values     |

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
