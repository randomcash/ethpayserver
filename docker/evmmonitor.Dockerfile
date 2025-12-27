# evmmonitor - EVM chain payment monitor
#
# Build from ethpayserver directory:
#   docker build -f docker/evmmonitor.Dockerfile -t evmmonitor .

FROM rust:1.90.0-bookworm AS builder

ARG HOME
ENV HOME=$HOME

WORKDIR /usr/local/server/
COPY . .

# Remove local patch (use git dependencies instead)
RUN sed -i '/^\[patch\./,/^$/d' Cargo.toml

RUN \
    --mount=type=secret,id=GIT_AUTH_TOKEN \
    --mount=type=cache,target=./target \
    --mount=type=cache,target=$HOME/.cargo \
    if [ -f /run/secrets/GIT_AUTH_TOKEN ]; then \
        TOKEN=$(cat /run/secrets/GIT_AUTH_TOKEN) && \
        git config --global credential.helper store && \
        echo "https://x-access-token:${TOKEN}@gitlab.com" > ~/.git-credentials && \
        git config --global url."https://x-access-token:${TOKEN}@gitlab.com/".insteadOf "https://gitlab.com/" ; \
    fi && \
    cargo install --root build --path evm --bin evmmonitor --features monitor-bin

FROM debian:bookworm-slim AS runtime
RUN DEBIAN_FRONTEND=noninteractive apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y -q ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false evmmonitor
COPY --from=builder /usr/local/server/build/bin/evmmonitor /usr/local/bin/
USER evmmonitor

# Required: EVMMONITOR_REDIS_URL, EVMMONITOR_CHAINS, EVMMONITOR_CHAIN_{ID}_RPC_HTTP
ENV RUST_LOG=info

ENTRYPOINT ["evmmonitor"]
