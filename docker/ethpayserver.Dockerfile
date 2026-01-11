# ethpayserver - Main API server
#
# Build from ethpayserver directory:
#   docker build -f docker/ethpayserver.Dockerfile -t ethpayserver .

FROM rust:1.90.0-bookworm AS builder

ARG HOME
ENV HOME=$HOME

WORKDIR /usr/local/server/
COPY . .

# Remove local patch (use git dependencies instead)
# Also exclude client from workspace (it has local path deps to ui-kit)
RUN sed -i '/^\[patch\./,/^$/d' Cargo.toml && \
    sed -i 's/"client"//' Cargo.toml

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
    cargo install --root build --path server --bin ethpayserver && \
    cargo install --root build --path server --bin migrate_postgres

FROM debian:bookworm-slim AS runtime
RUN DEBIAN_FRONTEND=noninteractive apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y -q ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false ethpayserver
COPY --from=builder /usr/local/server/build/bin/ethpayserver /usr/local/bin/
COPY --from=builder /usr/local/server/build/bin/migrate_postgres /usr/local/bin/
USER ethpayserver

# Required: DATABASE_URL, REDIS_URL
ENV RUST_LOG=info
ENV HOST=0.0.0.0
ENV PORT=3000
ENV ENABLE_SWAGGER=true

EXPOSE 3000
ENTRYPOINT ["ethpayserver"]
