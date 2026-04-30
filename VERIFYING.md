# Verifying random.cash Releases

Every release artifact (Docker image, binary tarball, git tag) is
cryptographically signed so you can verify it was produced by the
authorized CI pipeline from a known commit.

## Prerequisites

Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/):

```sh
# Arch Linux
pacman -S cosign

# macOS
brew install cosign

# Binary
curl -sLO https://github.com/sigstore/cosign/releases/latest/download/cosign-linux-amd64
chmod +x cosign-linux-amd64 && sudo mv cosign-linux-amd64 /usr/local/bin/cosign
```

## Docker images

All images pushed from the CI pipeline are signed with cosign in keyless
(OIDC) mode. The identity is the GitLab CI job that built the image,
recorded in the Sigstore transparency log.

```sh
# Server
cosign verify \
  --certificate-identity-regexp "https://gitlab.com/random.cash/ethpayserver//.gitlab-ci.yml@refs/(heads|tags)/.*" \
  --certificate-oidc-issuer "https://gitlab.com" \
  registry.gitlab.com/random.cash/ethpayserver:<tag>

# EVM monitor
cosign verify \
  --certificate-identity-regexp "https://gitlab.com/random.cash/ethpayserver//.gitlab-ci.yml@refs/(heads|tags)/.*" \
  --certificate-oidc-issuer "https://gitlab.com" \
  registry.gitlab.com/random.cash/ethpayserver/evmmonitor:<tag>

# Checkout client
cosign verify \
  --certificate-identity-regexp "https://gitlab.com/random.cash/ethpayserver//.gitlab-ci.yml@refs/(heads|tags)/.*" \
  --certificate-oidc-issuer "https://gitlab.com" \
  registry.gitlab.com/random.cash/ethpayserver/client:<tag>
```

Replace `<tag>` with the short commit SHA or branch-latest tag
(e.g. `main-latest`, `a1b2c3d`).

## Binary tarballs

Tagged releases include signed binary tarballs on the
[GitLab releases page](https://gitlab.com/random.cash/ethpayserver/-/releases).
Each tarball ships with a `.sig` (signature) and `.cert` (certificate).

```sh
# Download artifacts
TAG="v0.1.0"  # replace with actual tag
curl -LO "https://gitlab.com/random.cash/ethpayserver/-/releases/${TAG}/downloads/server-${TAG}-x86_64-linux.tar.gz"
curl -LO "https://gitlab.com/random.cash/ethpayserver/-/releases/${TAG}/downloads/server-${TAG}-x86_64-linux.tar.gz.sig"
curl -LO "https://gitlab.com/random.cash/ethpayserver/-/releases/${TAG}/downloads/server-${TAG}-x86_64-linux.tar.gz.cert"

# Verify
cosign verify-blob \
  --certificate-identity-regexp "https://gitlab.com/random.cash/ethpayserver//.gitlab-ci.yml@refs/tags/.*" \
  --certificate-oidc-issuer "https://gitlab.com" \
  --signature "server-${TAG}-x86_64-linux.tar.gz.sig" \
  --certificate "server-${TAG}-x86_64-linux.tar.gz.cert" \
  "server-${TAG}-x86_64-linux.tar.gz"
```

The same pattern applies to `evmmonitor-<tag>-x86_64-linux.tar.gz`.

## Git tags

Release tags are GPG-signed by a project maintainer:

```sh
git fetch --tags
git tag -v <tag>
```

The maintainer key fingerprint is published below.

### Maintainer signing keys

| Maintainer | Fingerprint |
|------------|-------------|
| randomcash | *(publish after first signed tag)* |

## What the signatures prove

| Artifact | Guarantee |
|----------|-----------|
| Docker image | Built by the `random.cash/ethpayserver` GitLab CI pipeline from the commit identified by the image tag. Recorded in the Sigstore public transparency log. |
| Binary tarball | Produced by the release pipeline for a specific git tag. Signature binds the tarball contents to the CI identity that created it. |
| Git tag | Created by a project maintainer holding the listed GPG key. |
