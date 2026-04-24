# Production Gate: testnet to mainnet promotion

This document defines the procedure for promoting an ethpayserver release from
testnet to production (mainnet), including the manual approval gate, post-deploy
health verification, and rollback playbook.

## Strategy

**Deploy + post-deploy smoke + manual rollback.** random.cash runs on a single
VPS via docker-compose. There is no blue/green or canary infrastructure. Safety
comes from:

1. A human approval gate in CI before the production deploy triggers.
2. A health gate that verifies the new image before declaring success.
3. A documented, single-command rollback path.

## Pre-promotion checklist

Before clicking the manual deploy button in CI for the `main` branch:

- [ ] All chain monitors are synced on testnet (`/health/deep` shows
      `data_fresh: true` and all RPCs report `ok`).
- [ ] RPC provider quotas are sufficient for production traffic.
- [ ] All secrets are present in the production `.env` file:
      `DATABASE_URL`, `REDIS_URL`, `EVMMONITOR_CHAIN_*_RPC_*`,
      `WEBAUTHN_RP_ID`, `WEBAUTHN_RP_ORIGIN`.
- [ ] Database migrations have been reviewed. Run `migrate_postgres` in
      dry-run mode if available, or inspect the pending migration files
      for destructive operations (column drops, table renames).
- [ ] The testnet smoke test (`scripts/smoke-prod.sh` against testnet)
      is green.
- [ ] No open MRs with the `do-not-merge` label targeting `main`.

## Manual approval step

The `notify:deploy` CI job for the `main` branch is configured with
`when: manual`. This means the pipeline will pause at the notify stage
and wait for a maintainer to click "Run" in the GitLab UI.

The testnet deploy remains automatic — only production requires manual
approval.

### How it works in CI

```yaml
notify:deploy:
  stage: notify
  rules:
    - if: $CI_COMMIT_BRANCH == "testnet"        # auto
    - if: $CI_COMMIT_BRANCH == "main"
      when: manual                               # human clicks "Run"
      allow_failure: false                        # pipeline stays blocked
```

## Post-deploy health gate

After the deploy trigger fires, the `post-deploy:health-gate` CI job
polls the deployed instance's `/health/deep` endpoint for up to 60
seconds (configurable via `HEALTH_TIMEOUT`).

The gate passes when ALL of the following are true:

1. `/health/deep` returns HTTP 200.
2. `build_sha` in the response matches `$CI_COMMIT_SHORT_SHA`.
3. Postgres reports `status: "ok"`.
4. Redis reports `status: "ok"`.
5. All RPC chains report `status: "ok"` (no chain in error, disconnected,
   or connecting state).

If the gate does not pass within the timeout, the job fails. Because
Docker Compose keeps the old container running until the new one passes
its own health check, a failed gate means the old version is still
serving traffic — no rollback is needed in this case.

### Post-deploy smoke test

After the health gate passes, `post-deploy:smoke` runs the full smoke
test suite (`scripts/smoke-prod.sh`) against the deployed instance:

- `/health/live` — process is running
- `/health/ready` — DB, Redis, and all RPC chains reachable
- `/health/deep` — detailed dependency check
- Invoice create/read cycle via API key
- Checkout page load for the created invoice

### Required CI variables

| Variable | Description |
|----------|-------------|
| `DEPLOY_HEALTH_URL` | Full URL to `/health/deep` on the target env |
| `DEPLOY_SMOKE_URL` | Base URL for smoke tests (e.g. `https://api.random.cash`) |
| `DEPLOY_SMOKE_API_KEY` | API key with invoice create/read permissions |
| `DEPLOY_SMOKE_STORE_ID` | Store UUID the smoke API key is scoped to |

## Container registry tagging

Every CI pipeline tags container images with both the short commit SHA
and a branch-latest tag:

```
registry.gitlab.com/random.cash/ethpayserver:<short_sha>
registry.gitlab.com/random.cash/ethpayserver:<branch>-latest
```

The SHA tag is immutable and deterministic — it is the tag used for
rollback.

## Rollback procedure

If production is broken after a deploy, rollback to the previous known-good
image:

### 1. Identify the previous good SHA

```bash
# On the VPS, check which image was running before:
docker inspect ethpayserver_server --format='{{.Config.Image}}'
# Or check GitLab CI for the last successful main pipeline's short SHA.
```

### 2. Retag and redeploy

```bash
# SSH into the VPS
cd /path/to/ethpayserver/docker

# Update the image tag in .env or docker-compose override:
export SERVER_IMAGE=registry.gitlab.com/random.cash/ethpayserver:<previous_sha>
export MONITOR_IMAGE=registry.gitlab.com/random.cash/ethpayserver/evmmonitor:<previous_sha>
export CLIENT_IMAGE=registry.gitlab.com/random.cash/ethpayserver/client:<previous_sha>

# Pull and restart
docker compose -f docker-compose.prod.yml pull
docker compose -f docker-compose.prod.yml up -d
```

### 3. Verify the rollback

```bash
curl -s https://api.random.cash/health/deep | python3 -c \
  "import json,sys; d=json.load(sys.stdin); print(f'sha={d[\"build_sha\"]} pg={d[\"postgres\"][\"status\"]} redis={d[\"redis\"][\"status\"]}')"
```

Expected output: `sha=<previous_sha> pg=ok redis=ok`

### 4. Revert the commit on main

```bash
git revert <bad_commit_sha>
git push origin main
```

This prevents the bad commit from being accidentally re-deployed on the
next pipeline run.

## Build SHA verification

The `/health/deep` endpoint exposes a `build_sha` field that contains the
short commit SHA baked into the binary at compile time. This allows:

- The health-gate script to confirm the new version is actually running.
- The verifier pass (RCS-113) to match deployed commits to Linear issues.
- Operators to quickly confirm which version is live.

```json
{
  "build_sha": "abc1234",
  "version": "0.1.0",
  "postgres": { "status": "ok", "latency_ms": 3 },
  "redis": { "status": "ok", "latency_ms": 1 },
  "rpcs": { "1": { "status": "ok", "latency_ms": 12, "last_block": 20000000 } },
  "monitor": { "status": "ok", "data_fresh": true }
}
```
