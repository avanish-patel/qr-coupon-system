# Multi-stage build for minimal size
FROM rustlang/rust:nightly-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev sqlite-dev openssl-dev pkgconfig

WORKDIR /app

# Copy manifests
COPY Cargo.toml ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs

# Build dependencies (this will be cached)
RUN cargo build --release
RUN rm -rf src

# Copy actual source code
COPY src ./src
COPY static ./static

# Build the actual application
RUN touch src/main.rs && \
    cargo build --release

# Runtime stage - minimal Alpine
FROM alpine:3.19

# Install only runtime dependencies
RUN apk add --no-cache libgcc sqlite-libs

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/qr-coupon-system .
COPY --from=builder /app/static ./static

# Create data directory for SQLite with proper permissions
RUN mkdir -p /app/data && \
    chmod 777 /app/data

# Expose port
EXPOSE 3000

# Run the application
CMD ["./qr-coupon-system"]
