# Build stage for supervisor binary
FROM rust:1.89-slim AS rust-builder

WORKDIR /build
COPY supervisor/ /build/
RUN 
RUN cargo build --release

# Final stage
FROM python:3.12-slim

# TODO: remove unused packages later
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    git \
    openssh-client \
    rsync \
    curl \
    netcat-openbsd \
    xz-utils \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /scripts /webhook
COPY VERSION /scripts/VERSION
COPY src/check-push.sh /scripts

# Copy supervisor binary from build stage
COPY --from=rust-builder /build/target/release/git-supervisor /git-supervisor
RUN chmod +x /git-supervisor

# Webhook: install deps and copy script (no wheel build)
RUN pip install --no-cache-dir "flask>=3.0.3"
COPY gh-webhook/hook.py /webhook/hook.py

WORKDIR /webhook
CMD ["python3", "/webhook/hook.py"]