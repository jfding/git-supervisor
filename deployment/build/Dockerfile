# Build stage for supervisor binary
FROM rust:1.89-slim AS rust-builder

WORKDIR /build
COPY supervisor/ /build/
COPY src/check-push.sh /build/embed/check-push.sh
COPY VERSION /build/VERSION
RUN cargo build --release

# Final stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    git \
    openssh-client \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy supervisor binary from build stage
COPY --from=rust-builder /build/target/release/git-supervisor /git-supervisor
RUN chmod +x /git-supervisor

WORKDIR /work

CMD ["/git-supervisor", "watch", "--interval", "${TIMER_INTERVAL:-120}"]