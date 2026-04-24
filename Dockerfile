# Stage 1: Build
FROM rust:1.87-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .
RUN cargo build --release

# Stage 2: Runtime (Optimized for Home Assistant)
# Using the {arch} placeholder allows HA to automatically pick the right base image
FROM homeassistant/{arch}-base:latest

# Copy binary from builder
COPY --from=builder /usr/src/app/target/release/zeroclaw-coordinator-mcp /usr/local/bin/

# Copy add-on files
# Note: bashio is already included in the homeassistant base image
COPY run.sh /
RUN chmod a+x /run.sh

CMD [ "/run.sh" ]
