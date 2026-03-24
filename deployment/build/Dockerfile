# Build stage for supervisor binary
FROM rust:1.89-slim AS rust-builder

WORKDIR /build
COPY supervisor/ /build/
COPY src/check-push.sh /build/embed/check-push.sh
COPY VERSION /build/VERSION
RUN cargo build --release

# Final stage
FROM python:3.12-slim

# TODO: remove unused packages later
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    git \
    openssh-client \
    curl \
    netcat-openbsd \
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Copy supervisor binary from build stage
COPY --from=rust-builder /build/target/release/git-supervisor /git-supervisor
RUN chmod +x /git-supervisor

WORKDIR /scripts
COPY supervisor/deployments.sample.yaml /scripts/deployments.yaml
COPY src/check-push.sh /scripts
COPY VERSION /scripts/VERSION

CMD ["/git-supervisor", "hook", "--script", "/scripts/check-push.sh"]