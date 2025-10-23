# Build stage
FROM rust:bookworm AS builder

WORKDIR /app

# Download Tailwind CLI
RUN curl -sLO https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64 && \
    chmod +x tailwindcss-linux-x64 && \
    mv tailwindcss-linux-x64 tailwindcss

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy main.rs and lib.rs to build dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release && \
    rm -rf src target/release/federation target/release/federation.d target/release/deps/federation-* target/release/libfederation.* target/release/deps/libfederation-*

# Copy source code and Tailwind config
COPY src ./src
COPY migrations ./migrations
COPY styles ./styles
COPY tailwind.config.js ./
COPY askama.toml ./

# Build Tailwind CSS
RUN ./tailwindcss -i ./styles/input.css -o ./src/web/static/css/output.css --minify

# Build application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install required runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN useradd -m -u 1000 app

# Create data directories
RUN mkdir -p /app/data/index && \
    chown -R app:app /app

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/federation /usr/local/bin/federation

# Copy migrations
COPY --from=builder /app/migrations /app/migrations

# Copy static files (including compiled CSS)
COPY --from=builder /app/src/web/static /app/src/web/static

# Copy templates
COPY --from=builder /app/src/web/templates /app/src/web/templates

# Expose port (default, can be overridden by env var)
EXPOSE 3100

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:${PORT:-3100}/health || exit 1

# Run the binary
CMD ["federation", "serve"]
