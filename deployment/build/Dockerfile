# Build stage for supervisor binary
FROM rust:1.94-slim AS rust-builder

RUN rustup target add x86_64-unknown-linux-musl && \
    apt-get update && apt-get install -y musl-tools

WORKDIR /build
COPY . /build/

RUN  cargo build --release --target x86_64-unknown-linux-musl

# Final stage
FROM alpine:3.23

RUN apk add --no-cache git openssh-client tzdata ca-certificates bash

# Copy supervisor binary from build stage
COPY --from=rust-builder /build/target/x86_64-unknown-linux-musl/release/git-supervisor /git-supervisor
RUN chmod +x /git-supervisor

WORKDIR /work

# avoid the first time git fetch error without known_hosts in .ssh
ENV GIT_SSH_COMMAND="ssh -o StrictHostKeyChecking=no"

ENV TIMER_INTERVAL=120
ENV WEBHOOK_PORT=9870
CMD ["/bin/sh", "-c", "exec /git-supervisor watch --interval \"${TIMER_INTERVAL:-120}\" --webhook-port \"${WEBHOOK_PORT:-9870}\""]
