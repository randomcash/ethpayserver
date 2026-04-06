# evmmonitor - EVM chain payment monitor
#
# Expects pre-built binary from CI artifacts:
#   target/release/evmmonitor

FROM debian:bookworm-slim
RUN DEBIAN_FRONTEND=noninteractive apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y -q ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false evmmonitor
COPY target/release/evmmonitor /usr/local/bin/
USER evmmonitor

ENV RUST_LOG=info

ENTRYPOINT ["evmmonitor"]
