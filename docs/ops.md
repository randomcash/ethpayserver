# Operations Runbook

## Health Endpoints

ethpayserver exposes three health probe endpoints, none of which require authentication. They do **not** query invoice or payment tables — they only check infrastructure dependencies.

### `GET /health/live`

**Purpose:** Kubernetes/systemd liveness probe. Confirms the process is alive.

- Always returns **200 OK** if the server is running.
- If this fails, the process is unresponsive — restart it.

### `GET /health/ready`

**Purpose:** Kubernetes/systemd readiness probe. Gates traffic during rolling restarts.

Returns **200** only when **all** of the following respond within 1 second:

| Dependency | What is checked |
|------------|-----------------|
| Postgres   | `SELECT 1` ping |
| Redis      | `PING` command  |
| RPC chains | Each configured chain reports `is_healthy` via evmmonitor health data in Redis |

**200 response:**

```json
{"status": "ready"}
```

**503 response** (with list of failing dependencies):

```json
{"status": "not_ready", "failing": ["postgres", "rpc:56"]}
```

Use this endpoint for load-balancer health checks and deployment readiness gates.

### `GET /health/deep`

**Purpose:** Operator diagnostic endpoint. Not intended for load-balancer decisions — use `/health/ready` for that.

Always returns **200** with a JSON body containing per-dependency status and latencies:

```json
{
  "postgres": {"status": "ok", "latency_ms": 3},
  "redis": {"status": "ok", "latency_ms": 1},
  "rpcs": {
    "1":  {"status": "ok", "latency_ms": 5, "last_block": 20000000},
    "56": {"status": "error", "latency_ms": 1000, "error": "disconnected"}
  },
  "monitor": {"status": "ok", "data_fresh": true}
}
```

| Field | Description |
|-------|-------------|
| `postgres.latency_ms` | Round-trip time for a Postgres `SELECT 1` |
| `redis.latency_ms` | Round-trip time for a Redis `PING` |
| `rpcs.<chain_id>.last_block` | Latest block number reported by evmmonitor for this chain |
| `rpcs.<chain_id>.error` | Present only when the chain is unhealthy |
| `monitor.data_fresh` | `true` if evmmonitor has published health data to Redis |

### Existing admin-only endpoints

These require a `Bearer` token with server admin privileges:

- `GET /health/chains` — detailed per-chain health from evmmonitor (same data as `/health/deep` RPCs section, but includes watched address counts)
- `GET /metrics` — Prometheus exposition format for scraping

## Watching `/health/deep` between deploys

A 200 from `/health/live` or `/health/ready` does not mean payments are being
detected — the process can be up while `monitor.data_fresh` is false or an RPC
has gone quiet, and a check that only reads the status code stays green
through that. Two things exist so far that touch `/health/deep`, and both are
deploy-triggered, not continuous:

- `scripts/health-gate.sh`, run by `deploy-verify-testnet` in `ci.yml` on every
  push to `testnet`, and manually against mainnet per
  `docs/deployment/mainnet-gate.md`. Gates a rollout; does nothing once the
  rollout has succeeded.
- `scripts/smoke-prod.sh`, run manually per the same checklist.

Between deploys, coverage is `.github/workflows/health-monitor.yml`: it runs
`scripts/check-health-deep.sh` against both `testnet.random.cash` and
`pay.random.cash` every 5 minutes on a GitHub-hosted runner, asserts
`postgres`, `redis`, `monitor.data_fresh` and every `rpcs.*.status`, and checks
in to a Sentry Cron Monitor (`SENTRY_CRON_HEALTH_TESTNET_URL` /
`SENTRY_CRON_HEALTH_MAINNET_URL`) so a missed or failing check pages through
Sentry's alerting rather than sitting unread in the Actions tab. The scheduled
e2e workflow's dead-man's-switch moved onto the same vendor, checking in to
`SENTRY_CRON_E2E_URL`.

Both Sentry URLs are **required**, not optional. A watchdog that quietly skips
the check-in when its secret is unset would pass, deploy, and run indefinitely
detecting real outages while paging nobody — a dashboard nobody is watching is
not alerting, and neither is a red Actions run nobody has that tab open for.
Until the two monitors below exist and the secrets are set,
`health-monitor.yml` fails on **every** run — loudly, in the Actions tab, on a
5-minute cycle — rather than silently degrading to a no-op. That is
deliberate: it is the loudest signal code in this repo can produce for "the
alert path is not wired up yet," short of actually wiring it up, which needs
a human with Sentry dashboard access this repository does not have.

`rpcs.*.status` alone misses a chain whose indexer has wedged while the RPC
connection itself stays up — `status: ok` with `last_block` frozen. Each job
restores `.health-state/<env>.json` from an `actions/cache` entry keyed on the
environment (an ordinary GitHub Actions run has no other persistence between
schedule ticks), passes it to `check-health-deep.sh` as `HEALTH_STATE_FILE`,
and saves it back afterwards regardless of pass/fail. The script tracks how
many consecutive checks a chain's `last_block` has repeated and fails once
that exceeds `STALL_THRESHOLD` (default 3 — i.e. ~15-20 minutes flat at the
5-minute cadence).

Five minutes is GitHub Actions' practical floor, not the 30-60s this ticket
asked for — schedule intervals shorter than that are not reliable, and GitHub
can delay a scheduled run further under load. The two Sentry Cron Monitor
URLs above have to be created by hand in Sentry (Crons → new monitor → "check
in via HTTP") and the resulting URLs stored as repo secrets; that account
setup is outside what a commit here can do.

For the faster cadence, also add a Sentry **Uptime Check** (not a Cron
Monitor) against `/api/health/deep`, run from Sentry's own checkers at 30-60s.
As of this writing it was not confirmed whether Sentry's uptime check can
assert on the response body rather than just the status code — if it can,
point its alert at the same field failures; if it can't, the workflow above is
the fallback that actually reads the body, and is why it stays regardless of
what the uptime check can do.

## Editing a migration that has already run

`sqlx` checksums the **whole migration file** — SHA-384 of its bytes — and
records it in `_sqlx_migrations`. On the next run it compares, and refuses with
`VersionMismatch` if the file has changed. A one-word comment edit is enough.
Because `migrate` runs before `server` in the deploy, that failure keeps the
whole stack down.

Two facts worth knowing before you touch one:

- **Renaming a migration file is free.** The version is the numeric prefix and
  the description is stored but never compared, so only the bytes matter.
- **Editing its contents is not**, whether or not any statement changed.

When an edit is deliberate and the migration has already run somewhere, refresh
the recorded checksums rather than resetting the database:

```sh
# 1. Keep the current values — see "this is one-way" below.
psql "$DATABASE_URL" -c "\copy (SELECT version, encode(checksum,'hex') \
  FROM _sqlx_migrations ORDER BY version) TO 'checksums-before.csv' CSV"

# 2. Look at what is recorded.
psql "$DATABASE_URL" -c 'SELECT version, description FROM _sqlx_migrations ORDER BY version'

# 3. Apply.
psql "$DATABASE_URL" -f ops/refresh-migration-checksums.sql
```

`ops/refresh-migration-checksums.sql` re-records checksum and description per
version. It touches no schema and no data, and matches nothing on a database
where those migrations have not run. Regenerate it whenever a migration file's
bytes change.

No `-1`: the script carries its own `BEGIN`/`COMMIT` and sets
`ON_ERROR_STOP`. Without `ON_ERROR_STOP`, psql runs on past a failed `UPDATE`,
the enclosing transaction is already aborted, `COMMIT` degrades to a rollback —
and **psql still exits 0**, so a refresh that did nothing reports success and
the deploy goes down anyway.

### This is one-way

`sqlx::migrate!` bakes checksums into the binary at compile time, so an image
built before the edit carries the old ones. After the refresh, that image's
`migrate` exits `VersionMismatch` and `server` never starts behind it — so
**rolling back to any earlier image stops working**, and rollback is otherwise
just a `compose up` with an older tag. Run the refresh only alongside deploying
an image built from a tree that contains the edited migrations, and keep
`checksums-before.csv` if you might need to go back.

### Every environment, not just the one in front of you

Run it wherever the database has those migrations applied. Nothing in CI does it
for you, and `main` dispatches testnet while tags dispatch mainnet, so an
edit that only got refreshed on testnet takes the next environment down at the
first already-applied version it reaches.
