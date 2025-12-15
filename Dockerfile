# Multi-stage build for optimal image size
# Stage 1: Build the application
FROM rust:1.83-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a new empty shell project
WORKDIR /app

# Copy manifests
COPY Cargo.toml ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs

# Build dependencies - this will be cached if Cargo.toml doesn't change
RUN cargo build --release && \
    rm -rf src target/release/deps/vk_gateway*

# Copy the actual source code
COPY src ./src
RUN touch src/main.rs
# Build the actual application
RUN cargo build --release

# Stage 2: Create the runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN useradd -m -u 1000 appuser

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/vk-gateway /app/vk-gateway

# Change ownership to non-root user
RUN chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Expose the port (Cloud Run uses PORT environment variable)
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/bin/sh", "-c", "wget --no-verbose --tries=1 --spider http://localhost:${PORT:-8080}/health || exit 1"]

# Run the binary
CMD ["/app/vk-gateway"]
