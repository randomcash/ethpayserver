# Session log

Newest first. One entry per session. The point of this file is that the next
session does not have to reconstruct what happened from the diff — write down
what you learned, not only what you changed.

---

## 2026-09-26 — F001: the watch set the database expects, against the one the monitor holds

**Feature:** F001. Detection only — both directions, reported separately.

**Result:** pass. A new module diffs what the database expects to be watched
against what the monitor's own live-watch record says it holds, and returns two
lists that are never added together: `stale` (held, not expected — wasteful) and
`missing` (expected, not held — **a payment can arrive and be credited to
nobody**). Nothing is cleared or repaired: unwatching is an external side effect
with no rollback, so a reconciler that acted on a comparison it had got wrong
could stop watching an address about to be paid into.

Files:

- `data-service/src/postgres/watch_reconciliation.rs` — `WatchKey`,
  `WatchDivergence`, `compare_watches`, `reconcile_watches`, and
  `PgDataService::get_expected_watches`.
- `data-service/src/postgres/watch_reconciliation/tests.rs` — six unit tests on
  the comparison itself.
- `server/tests/watch_reconciliation.rs` — the integration test, `#[ignore]`d,
  needs `DATABASE_URL` and `TEST_REDIS_URL`.
- `data-service/src/postgres/mod.rs`, `server/Cargo.toml` — wiring, below.

**The key is the whole tuple: address, invoice, chain, optional token.** The
same address is watched once per asset, so an address can keep its native watch
and lose its token watch. Comparing on the address alone calls that healthy, and
a payment in that token then arrives uncredited. That is the direction that
loses money, so the test seeds exactly that case.

**Red before green, twice, against a real seeded discrepancy** in the local test
database and live-watch store this box already runs (the two the agent gate
passes in):

1. `compare_watches` stubbed to return an empty report — the state a reconciler
   is in when it truthfully says "0 discrepancies". Failure:
   `a watch the database expects and the monitor does not hold must be reported
   missing; missing=[]`.
2. the comparison narrowed to address + chain, dropping the token — the
   realistic version of the bug. Same assertion fired, this time printing a
   38-entry `missing` list that did **not** contain the seeded token watch.
   Three of the six unit tests went red with it.

Restoring each made both green again. Before that, the first run failed on the
fixture rather than the assertion (`column "asset_type" is of type asset_type
but expression is of type text` — a bound parameter needs `$5::asset_type`;
the neighbouring suite gets away without the cast because it uses a literal).

**Two things the next session should not have to rediscover:**

- **`lib.rs` cannot take another `pub mod` line.** `data-service/src/lib.rs` is
  432 lines, already over the 400-line limit, and the size gate fails a file
  that grows *while already over* — one line is enough. So the module is
  declared in `postgres/mod.rs` (319 lines) and the public path is
  `data_service::postgres::…`, not `data_service::…`. This is the first time the
  size gate has dictated *where* new code goes rather than how large it may be.
  Anyone adding a module to this crate hits it.
- **The live-watch reader is behind a non-default feature.** data-service's
  `redis` feature is enabled by exactly one thing in this workspace — `evm`'s
  `monitor-bin` — which is not a default feature. So `cargo test -p data-service`
  cannot see `RedisDataService` at all, and an integration test placed in
  `data-service/src/postgres/integration_tests/` would have been silently
  compiled out of CI's `-p data-service` run: a test that cannot fail. That is
  why this one lives in `server/tests/` with `redis` added to *server's
  dev-dependency* on data-service — test builds only; the running server still
  links data-service without it. Side effect worth knowing: with that feature
  now unified into the workspace test build, `cargo test --workspace` also
  compiles and runs the six pre-existing unit tests in data-service's live-watch
  module, which no local command ran before.

**Reachability, plainly: nothing calls this yet.** There is no endpoint and no
scheduled job — it is detection logic with a test that drives it the way a
caller would (construct both services, read both sides, compare), and no more
than that. `pub` is not reachability, and this repository has shipped three
pieces wired to nothing with green tests. Wiring an operator-visible report is
the next step, not a formality.

**What "expected" means, and why it is not narrower.** `get_expected_watches`
selects `watched_addresses` rows with `is_active = TRUE`, joined through
`payment_options` to `invoices`, and is scoped by nothing else. `is_active` is
the flag the system itself clears to record "no longer watched", so it is the
database's own statement of what should be watched; scoping by invoice status
would invent a policy no other query here applies, and a narrower expected set
*hides* missing watches. The known cost: cleanup unwatches before it deactivates
the row, so between those two steps a watch reads as `missing` while everything
is behaving. Such an entry belongs to an invoice that is no longer pending,
which is how it is told apart from the case that costs a payment.

**Fixture residue I left behind, deliberately reported:** the two ablation runs
panicked before their cleanup, leaving two users (with their stores, invoices,
payment options and active watch rows) and four live-watch keys in the shared
test fixtures. There is no database or live-store client on this box to remove
them by hand. They are inert — the fixtures already hold ~38 active watch rows
with no live watch from other suites, which is why the test asserts membership
of its own seeded keys rather than any total. A count assertion here would fail
for reasons that have nothing to do with the feature.

**`scripts/check.sh` does not pass on this box with `DATABASE_URL` and
`TEST_REDIS_URL` set, for two reasons that predate this change.** Recorded
because the script's own header invites you to set them and nothing here had
ever run that configuration, so the next session would read the failure as
theirs:

- `evm`'s three `#[ignore = "requires live RPC endpoint"]` provider tests fail
  with `HTTP error 525` from the endpoint. CI never runs them — its
  `--run-ignored` pass covers `data-service` and `server` only — so nothing has
  ever gated them, and this script is the first thing to try.
- `server`'s `the_create_wallet_endpoint_returns_addresses_to_verify_a_tron_key`
  fails with `409 this xpub is already registered to another account`. It
  registers a fixed xpub and needs a database where that xpub is not already
  registered; the shared test database has it. A test-isolation defect, not a
  product failure, and unrelated to this change — nothing here touches wallets.

Everything else in that configuration passed, the new test and every
`data-service` integration test included. With the two variables unset — the
configuration the baseline entry below recorded — `scripts/check.sh` exits 0.

One flake seen once and not explained: `evm/tests/monitor_recovery` failed in the
workspace step of one run and passed on a clean rerun of the same command. Its
output was truncated by the `run` helper (it tails 25 lines, which on a test
failure shows the doctest summary rather than the failing assertion — worth
knowing before trying to diagnose anything from that log). Recorded as a flake
observed once, cause not established.

**Next step:** a caller. Either an operator-readable report or a periodic check
that records the two counts separately; F002's counter is the cheap signal and
this is the real answer, so they belong together.

**Blockers:** none for F001. The 502 from the previous entry was not re-checked.

---

## 2026-09-26 — harness set up, baseline recorded

**Feature:** none. Scaffolding only.

**Result:** `memos/`, `AGENTS.md` and `scripts/check.sh` added. No product code
touched.

**Baseline, measured at `142d923` rather than assumed:**

```
cargo build --workspace                     clean      3m05s
cargo clippy --all-targets -- -D warnings   clean      1m41s
cargo test --workspace                      656 passed, 0 failed, 210 ignored
```

Nothing is failing. The interesting number is the 210 — roughly a quarter of the
suite does not run without a live Postgres and Redis, and CI gates on those. A
local green covers about three quarters of what blocks a merge.

**Next step:** F001. It is first because the failure it detects loses money: a
watch the database expects and the monitor does not hold means a payment arrives
and nobody notices.

**Blockers:**

- The API is returning 502. A deploy failed on an unhealthy dependency container
  while `/tmp` — a RAM-backed tmpfs — was 100% full with a stale 6.5 GB build
  directory in it. Clearing that needs privileges this session does not have.
- 14 of 20 open pull requests fail the line-limit gate. F003 is the sanctioned
  approach; doing it one file at a time is slower than deciding a sequence.

**Worth knowing, because each of these cost hours:**

- A check that has only ever been seen passing has not been shown to work. Two
  written today reported success while the thing they guarded was being
  violated — one because its test used an untracked file the check never looks
  at, one because it wrote its own counting marker on the path that was supposed
  not to count.
- An enforcement mechanism cannot enumerate what it conceals. A lint added to
  keep certain vocabulary out of this public repository shipped the terms and a
  file-by-file map of where they appear. It was reverted. The check belongs in
  the private repository.
- Deploy is not merge. A merge succeeded, its deploy failed, and the branch sat
  ahead of the running image for ninety minutes with every check green.
- A comment can assert an action its own code omits. One script wrote "left out
  of the queue deliberately" onto the very ticket it was failing to remove from
  the queue, six times.
- Ask what a reading is *of*. A query with a row limit, a diff against the wrong
  base, and a log that was the wrong place to look each produced a confident
  wrong answer within one day.
