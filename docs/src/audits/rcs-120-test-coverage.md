# RCS-120: Retroactive Test Coverage Audit

**Date:** 2026-05-22
**Scope:** All Done issues labeled `legacy:unverified` (23 total)

These issues were merged before the verification pipeline was added (2026-04-29).
This audit checked whether each feature has adequate test coverage and took action
accordingly.

## Summary

| Count | Action |
|-------|--------|
| 10    | Adequate tests — label removed |
| 3     | Partial tests — follow-up issue created, label removed |
| 10    | Infra/CI (no runtime test applicable) — label removed |
| **23** | **Total audited** |

## Results

### Must-have tier (money flow, auth, API endpoints)

| Issue | Feature | Tests? | Test count | Action |
|-------|---------|--------|------------|--------|
| RCS-74 | Refund and payout/settlement flow | PARTIAL | 2 (gas only) | Follow-up: RCS-144 |
| RCS-72 | Rate provider fiat-to-crypto | YES | ~25 | Label removed |
| RCS-75 | Webhook testing UI and delivery logs | PARTIAL | ~8 (service only) | Follow-up: RCS-145 |
| RCS-76 | Fantom and Gnosis chain support | YES | 9 | Label removed |
| RCS-64 | Passkey login fix | YES | ~15 | Label removed |
| RCS-60 | Registration endpoint fix | YES | ~8 | Label removed |
| RCS-98 | Health endpoints | YES | 10 | Label removed |
| RCS-67 | MCP server for agent payments | PARTIAL | 16 (math only) | Follow-up: RCS-146 |

### Nice-to-have tier

| Issue | Feature | Tests? | Test count | Action |
|-------|---------|--------|------------|--------|
| RCS-84 | Pagination UI | YES | 8 | Label removed |
| RCS-83 | Invoice list filters | YES | ~11 | Label removed |
| RCS-71 | WebSocket frontend wiring | YES | 7 | Label removed |
| RCS-48 | Create invoice form | YES | 8 | Label removed |
| RCS-61 | Logout button | YES | 2 | Label removed |

### Skip tier (infra/CI — no runtime test applicable)

| Issue | Feature | Action |
|-------|---------|--------|
| RCS-109 | Sentry wiring | Label removed |
| RCS-110 | Function size + coverage gating | Label removed |
| RCS-111 | Weekly cargo-mutants | Label removed |
| RCS-106 | clippy::unwrap\_used deny | Label removed |
| RCS-89 | Pre-commit githooks | Label removed |
| RCS-93 | Merge trains | Label removed |
| RCS-92 | mold linker + nextest | Label removed |
| RCS-91 | CI speed | Label removed |
| RCS-55 | Testnet deploy | Label removed |
| RCS-66 | Playwright perf timing | Label removed |

## Follow-up issues created

- **RCS-144** — test: add unit tests for refund and payout API handlers (from RCS-74)
- **RCS-145** — test: add tests for webhook delivery log endpoint and testing UI (from RCS-75)
- **RCS-146** — test: add tests for MCP server tool handlers and auth flow (from RCS-67)

## Gap details

### RCS-74: Refund/payout (biggest gap)

`server/src/api/refunds.rs` and `server/src/api/payouts.rs` contain handler functions
with no `#[cfg(test)]` blocks. Data-layer impls (`data-service/src/postgres/refund.rs`,
`payout.rs`) also have no unit tests. Only `evm/src/transaction.rs` has 2 gas-related
tests. No integration test covers the refund or payout flow.

### RCS-75: Webhook delivery logs

The webhook *service* (`server/src/services/webhook.rs`) has ~8 unit tests covering
retry logic, HMAC signing, and serialization. However, the delivery log query endpoint
and the webhook testing UI component have no tests.

### RCS-67: MCP server tool handlers

`mcp-server/src/server.rs` has 16 unit tests, but they only cover crypto conversion
math helpers. The MCP tool dispatch functions (`do_create_invoice`, `do_list_invoices`),
authentication flow, and end-to-end agent invoice creation are untested.
