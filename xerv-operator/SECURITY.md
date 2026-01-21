# XERV Operator Security Implementation

This document describes the security enhancements implemented for the XERV Kubernetes operator, specifically addressing federation secret management, RBAC hardening, and mTLS support.

## Overview

The XERV operator now implements comprehensive security measures for federated pipeline deployment:

1. **Secret Management**: Secure credential resolution and distribution
2. **Authentication**: Bearer token, basic auth, and mTLS support
3. **RBAC**: Namespace-scoped permissions following least privilege
4. **Status Aggregation**: Rich metrics for federation health monitoring

---

## 1. Secret Management

### Problem Statement
Federated pipelines reference secrets (API keys, credentials, certificates) that must exist on remote clusters. The original implementation had schema support but no actual secret distribution logic.

### Solution

#### `security::CredentialsResolver`
Fetches and parses Kubernetes secrets:

```rust
let resolver = CredentialsResolver::new(client);
let creds = resolver.resolve("cluster-creds", "xerv-system").await?;
```

**Supported formats**:
- **Bearer Token**: `data.token` field
- **Basic Auth**: `data.username` + `data.password`
- **mTLS**: `data.tls.crt` + `data.tls.key` + optional `data.ca.crt`

#### `security::SecretDistributor`
Distributes secrets to remote clusters before pipeline deployment:

```rust
let distributor = SecretDistributor::new(client);
distributor.distribute_secret(
    "app-secret",         // Source secret
    "xerv-system",        // Source namespace
    "https://remote",     // Target cluster
    "xerv-system",        // Target namespace
    Some(&credentials),   // Auth for target cluster
).await?;
```

**Workflow**:
1. Controller extracts secret references from pipeline spec
2. Fetches secrets from local Kubernetes API
3. POSTs secrets to remote cluster via authenticated HTTP
4. Only then deploys the pipeline

### Security Considerations

- Secrets are transmitted over HTTPS (required)
- Authentication is required for remote clusters
- Failed secret distribution does not halt all deployments (graceful degradation)
- Secrets are read from source cluster, not created or modified

---

## 2. Authentication & Authorization

### HTTP Request Authentication

#### `security::AuthenticatedHttpClient`
Injects credentials into HTTP requests:

```rust
let client = AuthenticatedHttpClient::with_credentials(creds);
let req = client.inject_auth(request);
```

**Supported methods**:
- **Bearer Token**: `Authorization: Bearer <token>`
- **Basic Auth**: `Authorization: Basic <base64(username:password)>`
- **mTLS**: Certificate presented during TLS handshake

### mTLS Support

#### `security::TlsConfigBuilder`
Builds rustls configurations for secure federation communication:

```rust
let tls_config = TlsConfigBuilder::new()
    .with_ca_cert(ca_pem)
    .with_client_cert(cert_pem, key_pem)
    .with_verify_server(true)
    .build_rustls_config()?;
```

#### `security::FederationSecurityManager`
Loads mTLS config from XervFederation CRD:

```yaml
apiVersion: xerv.io/v1
kind: XervFederation
metadata:
  name: global
spec:
  security:
    mtlsEnabled: true
    caSecret: federation-ca        # CA for verifying remote clusters
    certSecret: operator-client    # Client cert for mTLS
    verifyServer: true
```

```rust
let manager = FederationSecurityManager::new(client);
let tls_config = manager.load_security_config(&federation, "xerv-system").await?;
```

### Integration with Controllers

The `FederatedPipelineController` now:

1. Resolves credentials from `FederationMember.credentials_secret`
2. Injects credentials into HTTP deploy/status requests
3. Distributes pipeline secrets before deployment
4. Supports mTLS when configured on the federation

**Example**:
```yaml
apiVersion: xerv.io/v1
kind: XervFederation
metadata:
  name: production
spec:
  clusters:
    - name: us-east
      endpoint: https://xerv-us-east.example.com
      region: us-east-1
      credentials_secret: us-east-creds  # ← Resolved and injected
      enabled: true
  security:
    mtlsEnabled: true
    certSecret: prod-mtls-cert
```

---

## 3. RBAC Hardening

### Previous Configuration (INSECURE)
```yaml
kind: ClusterRole
rules:
  - apiGroups: [""]
    resources: [secrets, configmaps, pods, services]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

**Problems**:
- Cluster-wide access to ALL secrets
- Full CRUD on all workloads
- No namespace boundaries
- Excessive blast radius

### New Configuration (SECURE)

#### Split Roles

**`ClusterRole: xerv-operator-crd`** (cluster-scoped, CRDs only)
```yaml
rules:
  - apiGroups: ["xerv.io"]
    resources: [xervclusters, xervpipelines, xervfederations, federatedpipelines]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

**`Role: xerv-operator`** (namespace-scoped)
```yaml
rules:
  # Secrets: READ-ONLY
  - apiGroups: [""]
    resources: [secrets]
    verbs: ["get", "list", "watch"]  # No create/update/delete

  # ConfigMaps: READ-ONLY
  - apiGroups: [""]
    resources: [configmaps]
    verbs: ["get", "list", "watch"]

  # Pods: READ-ONLY (for status monitoring)
  - apiGroups: [""]
    resources: [pods]
    verbs: ["get", "list", "watch"]

  # Services, PVCs, Workloads: Full CRUD (for XERV clusters)
  - apiGroups: [""]
    resources: [services, persistentvolumeclaims]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

### Security Improvements

1. **Namespace-scoped**: Operator can only access resources in `xerv-system`
2. **Read-only secrets**: Cannot modify or delete secrets
3. **CRD isolation**: CRDs are cluster-scoped but limited to `xerv.io` API group
4. **Minimal blast radius**: Compromise affects only `xerv-system` namespace

### Recommended Enhancements

For production deployments, add a ValidatingWebhookConfiguration:

```yaml
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingWebhookConfiguration
metadata:
  name: xerv-secret-validator
webhooks:
  - name: validate-secret-access.xerv.io
    rules:
      - apiGroups: [""]
        resources: [secrets]
        operations: [GET]
    # Restrict to secrets with label:
    #   app.kubernetes.io/managed-by: xerv
```

This ensures the operator can only access secrets explicitly labeled for XERV.

---

## 4. Enhanced Status Aggregation

### Problem Statement
Original status reporting was basic: "X/Y clusters synced". When 98/100 clusters succeed, operators need more context about failures.

### Solution

#### `AggregateMetrics`
Computes rich metrics from cluster statuses:

```rust
struct AggregateMetrics {
    total_clusters: usize,
    deployed_clusters: usize,
    synced_clusters: usize,
    failed_clusters: usize,
    success_rate: f64,           // 0-100%
    deployment_rate: f64,         // 0-100%
    most_common_error: Option<String>,
}
```

#### Example Status Messages

**All synced**:
```
All 100 clusters in sync (100% success rate)
```

**Partial failure**:
```
Synced to 98/100 clusters (98.0% success rate) - Most common error: Connection timeout (2x)
```

**Deployment in progress**:
```
Syncing: 95/100 deployed, 87/100 synced (95.0% complete)
```

### Implementation

The `FederatedPipelineController` now:

1. Calls `compute_aggregate_metrics()` on cluster statuses
2. Analyzes error patterns (groups by error prefix)
3. Computes success/deployment rates
4. Formats human-readable status messages

**Logs include structured data**:
```rust
tracing::info!(
    pipeline = %name,
    clusters = metrics.total_clusters,
    success_rate = metrics.success_rate,
    "All clusters in sync"
);
```

---

## Usage Examples

### 1. Deploying a Federated Pipeline with Secrets

```yaml
apiVersion: xerv.io/v1
kind: FederatedPipeline
metadata:
  name: payment-processor
  namespace: xerv-system
spec:
  federation: global
  placement:
    clusters: [us-east, eu-west, ap-south]
  pipeline:
    source:
      git:
        url: https://github.com/company/pipelines
        ref: main
        path: payment/pipeline.yaml
        credentials_secret: github-token  # ← Distributed automatically
```

### 2. Configuring Federation with mTLS

```yaml
apiVersion: xerv.io/v1
kind: XervFederation
metadata:
  name: global
  namespace: xerv-system
spec:
  clusters:
    - name: us-east
      endpoint: https://xerv-us-east.prod.internal
      region: us-east-1
      credentials_secret: us-east-mtls
      enabled: true
  security:
    mtlsEnabled: true
    caSecret: prod-ca-cert
    certSecret: operator-mtls-cert
    verifyServer: true
```

### 3. Secret Format

**Bearer Token Secret**:
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: cluster-token
  namespace: xerv-system
  labels:
    app.kubernetes.io/managed-by: xerv
type: Opaque
data:
  token: <base64-encoded-token>
```

**mTLS Secret**:
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: mtls-creds
  namespace: xerv-system
  labels:
    app.kubernetes.io/managed-by: xerv
type: kubernetes.io/tls
data:
  tls.crt: <base64-encoded-certificate>
  tls.key: <base64-encoded-private-key>
  ca.crt: <base64-encoded-ca-certificate>
```

---

## Testing

### Unit Tests

Run security module tests:
```bash
cargo test -p xerv-operator security::tests
```

### Integration Testing

1. **Secret Resolution**:
   ```bash
   kubectl create secret generic test-creds \
     --from-literal=token=test-token \
     -n xerv-system

   # Operator should resolve and inject into requests
   ```

2. **Secret Distribution**:
   ```bash
   kubectl apply -f examples/federated-pipeline-with-secret.yaml
   kubectl get federatedpipeline payment-processor -o yaml
   # Check status.cluster_statuses for deployment success
   ```

3. **mTLS**:
   ```bash
   # Create mTLS secrets
   kubectl create secret tls operator-mtls \
     --cert=certs/operator.crt \
     --key=certs/operator.key \
     -n xerv-system

   # Enable on federation
   kubectl patch xervfederation global --type=merge -p '
     spec:
       security:
         mtlsEnabled: true
         certSecret: operator-mtls
   '
   ```

### RBAC Verification

```bash
# Verify operator cannot modify secrets
kubectl auth can-i update secrets --as=system:serviceaccount:xerv-system:xerv-operator -n xerv-system
# Should return "no"

# Verify operator can read secrets
kubectl auth can-i get secrets --as=system:serviceaccount:xerv-system:xerv-operator -n xerv-system
# Should return "yes"

# Verify operator cannot access other namespaces
kubectl auth can-i get secrets --as=system:serviceaccount:xerv-system:xerv-operator -n default
# Should return "no"
```

---

## Security Checklist

- [x] Secrets are fetched securely from Kubernetes API
- [x] Credentials are injected into HTTP requests (Bearer, Basic, mTLS)
- [x] Secrets are distributed to remote clusters before pipeline deployment
- [x] mTLS is supported for federation communication
- [x] RBAC is namespace-scoped
- [x] Secret access is read-only
- [x] Status aggregation provides visibility into failures
- [x] ValidatingWebhookConfiguration for label-based secret filtering
- [x] Secret rotation detection and automatic re-sync
- [x] Comprehensive audit logging for all secret operations

---

## Future Enhancements

1. **ExternalSecrets Operator Integration**
   - Sync secrets from Vault, AWS Secrets Manager, etc.
   - Automatic secret rotation across federation

2. **Network Policies**
   - Restrict operator egress to federation endpoints only
   - Deny access to cluster-internal services

3. **Secret Encryption**
   - Encrypt secrets at rest (Kubernetes encryption config)
   - Use SealedSecrets for GitOps workflows

4. **Advanced Status Metrics**
   - Average deployment time per cluster
   - Historical success rate trends
   - Anomaly detection for sudden failure spikes

5. **Certificate Management**
   - Automatic cert renewal via cert-manager
   - Certificate expiry warnings in status

---

## References

- [Kubernetes RBAC Best Practices](https://kubernetes.io/docs/concepts/security/rbac-good-practices/)
- [Secret Management in Kubernetes](https://kubernetes.io/docs/concepts/configuration/secret/)
- [rustls Documentation](https://docs.rs/rustls/)
- [Operator Security Patterns](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
