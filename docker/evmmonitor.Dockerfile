# evmmonitor - EVM chain payment monitor
#
# Expects pre-built binary from CI artifacts:
#   target/release/evmmonitor

FROM archlinux:base
RUN pacman -Sy --noconfirm ca-certificates && pacman -Scc --noconfirm
RUN useradd -r -s /bin/false evmmonitor
COPY --chmod=755 target/release/evmmonitor /usr/local/bin/
USER evmmonitor

ENV RUST_LOG=info

ENTRYPOINT ["evmmonitor"]
