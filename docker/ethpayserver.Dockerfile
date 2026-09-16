# ethpayserver - Main API server
#
# Expects pre-built binaries from CI artifacts:
#   target/release/ethpayserver
#   target/release/migrate_postgres

FROM archlinux:base
RUN pacman -Sy --noconfirm ca-certificates && pacman -Scc --noconfirm
RUN useradd -r -s /bin/false ethpayserver
COPY --chmod=755 target/release/ethpayserver /usr/local/bin/
COPY --chmod=755 target/release/migrate_postgres /usr/local/bin/

# Installed plugins' wasm artifacts.
#
# Created here, owned by the user the server runs as, because Docker copies
# an image directory's ownership into a named volume only while that volume
# is still empty. Mounting a volume over a path the image does not create
# leaves it root:root, and this server does not run as root - so the
# directory has to exist, owned correctly, in the image that first mounts
# the volume. Do it later and the ownership is already fixed.
#
# The failure is invisible until something writes: a root-owned 0755
# directory reads fine, so the boot-time load works and only installing a
# plugin fails.
RUN mkdir -p /var/lib/ethpayserver/plugins \
 && chown -R ethpayserver:ethpayserver /var/lib/ethpayserver
USER ethpayserver

# Overridable, but correct by default: the repository default is ./plugins,
# which is right for a development run and wrong for a container, where it
# would put artifacts in the working directory and lose them on redeploy.
ENV ETHPAY_PLUGIN_DIR=/var/lib/ethpayserver/plugins
ENV RUST_LOG=info
ENV HOST=0.0.0.0
ENV PORT=3000
ENV ENABLE_SWAGGER=true

EXPOSE 3000
ENTRYPOINT ["ethpayserver"]
