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
psql "$DATABASE_URL" -c 'SELECT version, description FROM _sqlx_migrations ORDER BY version'
psql "$DATABASE_URL" -1 -f ops/refresh-migration-checksums.sql
```

`ops/refresh-migration-checksums.sql` re-records checksum and description per
version. It touches no schema and no data, and matches nothing on a database
where those migrations have not run. Regenerate it whenever a migration file's
bytes change.
