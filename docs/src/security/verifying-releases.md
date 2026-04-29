# Verifying Releases

random.cash signs every release artifact with [Sigstore cosign](https://docs.sigstore.dev/)
in keyless mode. No long-lived signing key exists; the CI pipeline's GitLab OIDC
identity is the signer, and every signature is recorded in the public
[Rekor transparency log](https://docs.sigstore.dev/logging/overview/).

## Quick start

Install cosign, then run the appropriate verify command for your artifact type.

### Docker images

```sh
cosign verify \
  --certificate-identity-regexp \
    "https://gitlab.com/random.cash/ethpayserver//.gitlab-ci.yml@refs/(heads|tags)/.*" \
  --certificate-oidc-issuer "https://gitlab.com" \
  registry.gitlab.com/random.cash/ethpayserver:<tag>
```

Three images are published per commit:

| Image | Reference |
|-------|-----------|
| Server (API + migrations) | `registry.gitlab.com/random.cash/ethpayserver:<tag>` |
| EVM monitor | `registry.gitlab.com/random.cash/ethpayserver/evmmonitor:<tag>` |
| Checkout client | `registry.gitlab.com/random.cash/ethpayserver/client:<tag>` |

Tags follow the pattern `<short-sha>` (immutable, e.g. `a1b2c3d`) or
`<branch>-latest` (rolling, e.g. `main-latest`).

### Binary tarballs

Tagged releases attach binary tarballs to the
[GitLab releases page](https://gitlab.com/random.cash/ethpayserver/-/releases).
Each tarball has a `.sig` and `.cert` companion file.

```sh
cosign verify-blob \
  --certificate-identity-regexp \
    "https://gitlab.com/random.cash/ethpayserver//.gitlab-ci.yml@refs/tags/.*" \
  --certificate-oidc-issuer "https://gitlab.com" \
  --signature server-<tag>-x86_64-linux.tar.gz.sig \
  --certificate server-<tag>-x86_64-linux.tar.gz.cert \
  server-<tag>-x86_64-linux.tar.gz
```

### Git tags

```sh
git tag -v <tag>
```

Release tags are GPG-signed by a project maintainer. See
[VERIFYING.md](https://gitlab.com/random.cash/ethpayserver/-/blob/main/VERIFYING.md)
at the repository root for the maintainer key fingerprint.

## How it works

1. The CI pipeline builds binaries and pushes Docker images.
2. A dedicated **sign** stage runs after every image push. Each sign job
   receives a short-lived OIDC token from GitLab (`id_tokens` / Sigstore
   audience) and calls `cosign sign --yes` (images) or
   `cosign sign-blob --yes` (tarballs).
3. Cosign exchanges the OIDC token for a short-lived code-signing
   certificate from [Fulcio](https://docs.sigstore.dev/certificate_authority/overview/)
   and records the signature in [Rekor](https://docs.sigstore.dev/logging/overview/).
4. The sign stage is **mandatory** -- the pipeline fails if any signing
   step fails. There is no path to publish an unsigned artifact.

## Supply chain summary

| Layer | Mechanism |
|-------|-----------|
| Source | GPG-signed git tags |
| Build | Reproducible Nix builds (RCS-94, in progress) |
| Container images | cosign keyless via GitLab OIDC |
| Binary tarballs | cosign sign-blob keyless via GitLab OIDC |
| Transparency | Sigstore Rekor public log |
