# ethpayserver - Main API server
#
# Expects pre-built binaries from CI artifacts:
#   target/release/ethpayserver
#   target/release/migrate_postgres

FROM debian:bookworm-slim
RUN DEBIAN_FRONTEND=noninteractive apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y -q ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false ethpayserver
COPY target/release/ethpayserver /usr/local/bin/
COPY target/release/migrate_postgres /usr/local/bin/
USER ethpayserver

ENV RUST_LOG=info
ENV HOST=0.0.0.0
ENV PORT=3000
ENV ENABLE_SWAGGER=true

EXPOSE 3000
ENTRYPOINT ["ethpayserver"]
