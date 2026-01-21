# XERV Dockerfile
#
# Multi-stage build for minimal production image.

# Build stage
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY xerv-core ./xerv-core
COPY xerv-nodes ./xerv-nodes
COPY xerv-executor ./xerv-executor
COPY xerv-cluster ./xerv-cluster
COPY xerv-cli ./xerv-cli
COPY src ./src

# Build release binary
RUN cargo build --release --bin xerv

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -u 1000 -m xerv

# Create data directory
RUN mkdir -p /data /pipelines && chown -R xerv:xerv /data /pipelines

# Copy binary from builder
COPY --from=builder /build/target/release/xerv /usr/local/bin/xerv

# Switch to non-root user
USER xerv

# Set working directory
WORKDIR /data

# Default environment
ENV XERV_API_PORT=8080 \
    XERV_DATA_DIR=/data \
    XERV_DISPATCH_BACKEND=memory \
    RUST_LOG=xerv=info

# Expose ports
EXPOSE 8080 9090

# Declare volume for persistent data (arena, WAL)
# Mount with: docker run -v xerv-data:/data ...
VOLUME ["/data"]

# Health check
HEALTHCHECK --interval=10s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Default command
ENTRYPOINT ["xerv"]
CMD ["serve"]
