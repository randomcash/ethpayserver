# ethpayserver - Main API server
#
# Expects pre-built binaries from CI artifacts:
#   target/release/ethpayserver
#   target/release/migrate_postgres

FROM archlinux:base
RUN pacman -Sy --noconfirm ca-certificates && pacman -Scc --noconfirm
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
