# Build stage for Rust binary
FROM rust:1.75-slim as rust-builder

WORKDIR /build
COPY src/check-push-rs /build
RUN cargo build --release

# Final stage
FROM python:3.12-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    git \
    openssh-client \
    rsync \
    curl \
    netcat-openbsd \
    xz-utils \
    nodejs  \
    npm  \
    && rm -rf /var/lib/apt/lists/*

# for github.com ssh access
COPY dot.ssh/* /root/.ssh/

RUN mkdir -p /scripts
COPY check-push.sh /scripts
COPY prod2latest.sh /scripts
COPY cleanup-archives.sh /scripts

# Copy Rust binary from build stage
COPY --from=rust-builder /build/target/release/check-push-rs /scripts/check-push-rs
RUN chmod +x /scripts/check-push-rs

WORKDIR /webhook
COPY hook.py /webhook
COPY *.whl /webhook
RUN pip install /webhook/*.whl

CMD /webhook/hook.py