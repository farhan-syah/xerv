# XERV Docker Setup

Docker Compose configurations for running XERV locally.

## Quick Start

### Basic (In-Memory Backend)

```bash
cd docker
docker compose up -d
```

API available at: http://localhost:8080

### With Redis Backend

```bash
docker compose -f docker-compose.yaml -f docker-compose.redis.yaml up -d
```

Includes:

- XERV with Redis Streams dispatch
- Redis 7 with persistence
- Redis Commander at http://localhost:8081 (add `--profile debug`)

### With NATS Backend

```bash
docker compose -f docker-compose.yaml -f docker-compose.nats.yaml up -d
```

Includes:

- XERV with NATS JetStream dispatch
- NATS 2 with JetStream enabled
- NATS monitoring at http://localhost:8222

## Configuration

### Environment Variables

| Variable                | Default     | Description                                         |
| ----------------------- | ----------- | --------------------------------------------------- |
| `XERV_DISPATCH_BACKEND` | `memory`    | Dispatch backend: `memory`, `raft`, `redis`, `nats` |
| `XERV_API_PORT`         | `8080`      | HTTP API port                                       |
| `XERV_DATA_DIR`         | `/data`     | Data directory for arena and WAL                    |
| `XERV_METRICS_ENABLED`  | `true`      | Enable Prometheus metrics                           |
| `RUST_LOG`              | `xerv=info` | Log level                                           |

### Redis-specific

| Variable                        | Default | Description          |
| ------------------------------- | ------- | -------------------- |
| `XERV_DISPATCH_REDIS_URL`       | -       | Redis connection URL |
| `XERV_DISPATCH_REDIS_STREAMS`   | `true`  | Use Redis Streams    |
| `XERV_DISPATCH_REDIS_POOL_SIZE` | `10`    | Connection pool size |

### NATS-specific

| Variable                       | Default       | Description         |
| ------------------------------ | ------------- | ------------------- |
| `XERV_DISPATCH_NATS_URL`       | -             | NATS connection URL |
| `XERV_DISPATCH_NATS_JETSTREAM` | `true`        | Use JetStream       |
| `XERV_DISPATCH_NATS_STREAM`    | `XERV_TRACES` | Stream name         |

## Volumes

| Volume        | Path         | Description                      |
| ------------- | ------------ | -------------------------------- |
| `xerv-data`   | `/data`      | Arena and WAL storage            |
| `./pipelines` | `/pipelines` | Pipeline definitions (read-only) |

## Debugging

Enable debug tools:

```bash
# Redis Commander
docker compose -f docker-compose.yaml -f docker-compose.redis.yaml --profile debug up -d

# NATS Box
docker compose -f docker-compose.yaml -f docker-compose.nats.yaml --profile debug up -d
docker compose exec nats-box nats stream ls
```

## Building

Build the XERV image locally:

```bash
docker compose build
```

## Stopping

```bash
docker compose down        # Stop containers
docker compose down -v     # Stop and remove volumes
```
