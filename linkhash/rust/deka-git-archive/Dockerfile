# Deka Git Server
# HTTP git server for website deployments with JWT authentication

FROM rust:1.82-slim as builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 tana && \
    mkdir -p /data/repos && \
    chown -R tana:tana /data

# Copy binary from builder
COPY --from=builder /build/target/release/deka-git /usr/local/bin/

USER tana
WORKDIR /data

# Environment variables with sensible defaults
ENV PORT=8508
ENV REPOS_DIR=/data/repos
ENV RUST_LOG=deka_git=info

EXPOSE 8508

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:${PORT}/health || exit 1

CMD ["deka-git"]
