# Session log

Newest first. One entry per session. The point of this file is that the next
session does not have to reconstruct what happened from the diff — write down
what you learned, not only what you changed.

---

## 2026-09-26 — F002: the post-delete unwatch counter, shown moving on both branches

**Feature:** F002. A test for a counter that already existed and had never been
seen fire.

**Result:** pass. Both branches of `unwatch_after_delete` that call
`record_unwatch_failed` are covered, and both were shown red on purpose before
they were green. `scripts/check.sh` exits 0 with the variables unset.

Files:

- `server/src/api/admin/deletion/unwatch_counter_tests.rs` — new, five unit
  tests on the counter.
- `server/src/api/admin/deletion/mod.rs` — the seam (below), the doc-comment
  repair (below), `cleanup_info` widened to `pub(super)` so the new module
  reuses the fixture instead of copying it.
- `server/src/api/admin/deletion/store.rs`, `server/src/api/users/deletion.rs` —
  the two call sites, one line each.
- `server/tests/admin_account_deletion/self_service.rs`,
  `.../support.rs` — the reachability test and its counter reader.
- `memos/features.json` — F002 `passes` only.

**Why a unit test, and why that is the stronger placement here.** The last
session's warning was about a test that CI compiles out; the mirror of it is a
test CI runs only in the pass that needs a database, where this suite's own
convention is `let Some(pg) = service().await else { return; }` — which reports
green when the variable is unset. These five need no database and no live-watch
store, so they run in the ordinary `cargo test --workspace` step and cannot be
skipped into a green by a missing variable. The claim they cannot make is
reachability, so that is a separate `#[ignore]`d test in `server/tests/`
(`-p server` is in CI's `--run-ignored` pass) driving the real `DELETE /users/me`
handler against a real database with no monitor wired.

**The seam, and why it was needed.** `unwatch_after_delete` took
`&PgAppState<A>`, which pins the monitor to `RedisEVMMonitor` — and that type
cannot be constructed at all without a reachable live-watch store, so there was
no way to produce a publish failure from a test. It now takes
`Option<&E>, E: EVMMonitor + ?Sized`, which is the only thing it ever read off
the state. Call sites pass `state.evm_monitor.as_deref()`. Nothing else changed
about it.

**Red before green, four ablations:**

1. per-address increment commented out →
   `a_publish_failure_is_counted_once_per_address_and_stops_nothing` failed,
   `left: 0, right: 2`, "each address whose unwatch could not be published must
   be counted". The other four stayed green, so the tests are not
   interchangeable.
2. no-monitor increment commented out →
   `no_monitor_wired_counts_every_watch_it_left_behind` failed, `left: 0,
   right: 3`. Again only that one.
3. same ablation, integration test → `left: 0, right: 1`. So the endpoint-level
   test depends on the counter and not merely on the delete succeeding.
4. the `unwatch_after_delete` CALL removed from `delete_account` → the same
   integration test failed identically. That is the reachability half: it fails
   if the handler stops reaching the branch at all, not only if the counter
   stops counting.

**What the counter cannot see, now written down in three places rather than
one.** `unwatch_address_by_chain_id` succeeds when the command is *published*.
Nothing acknowledges it, so a command published to a channel with no subscriber
increments nothing, and `a_published_unwatch_counts_nothing_even_though_nothing_acknowledged_it`
is named to stop a reader concluding a zero means the monitor acted.

**A third silent path, found by writing the tests and deliberately left
alone.** A row `parse_watch_target` rejects is `continue`d without a command
being built and without being counted, and it leaves exactly the same stale
watch. It is not counted on purpose: the realistic instance is a watch on a
non-EVM chain, which this process's monitor never held, so counting it would
report a failure that did not happen. A malformed address would be worth
counting and is indistinguishable from the non-EVM case at that point. Pinned by
`a_row_the_monitor_cannot_address_is_skipped_without_counting` and stated in
`record_unwatch_failed`'s docs, so the next reader does not have to rediscover
it from a zero. Separating the two needs the row to carry why it was rejected,
which is more than F002.

**Two doc comments were lying, and are the only non-test behaviourless changes
here.** `unwatch_after_delete` had no doc comment at all: its block had been
absorbed into `record_unwatch_failed`'s (no blank line between them), and it
still said "Only `hard_delete_store` calls this: `delete_user_account` and
self-service `delete_account` both refuse outright" — self-service stopped
refusing and started calling it when the narrowing landed. `delete_account`'s
own doc still promised a refusal on a still-watched address, and its OpenAPI 409
still advertised one. The admin route's identical text is correct and was left
alone; it really does still refuse.

**Reading the counter in a test.** Through a recorder local to the calling
thread, never the process-wide one — that can be installed once per process and
this crate's own metrics tests already contend for it. Thread-local means the
future has to be polled on the thread that installs it: `block_on` in the unit
tests, and `#[tokio::test]`'s current-thread runtime in the integration one. A
future polled elsewhere would record into the global recorder and the assertion
would read zero no matter what the code did — a test that cannot fail, arrived
at by accident.

**F002's `notes` field in `features.json` said "It is NOT yet covered by a test,
so it stays false" and has been rewritten.** It was left alone on the grounds
that the brief says to change only `passes`, which was the wrong call: the notes
then contradicted the flag beside them, and of the two the notes are the part a
reader believes, because they explain rather than assert. The rule means "do not
edit another feature's row", not "leave a statement standing once it has become
false". The replacement says what the tests cover and, as importantly, what a
zero on this counter still does not rule out.

**Pre-existing failures, re-measured rather than assumed, both unrelated:**

- `cargo test -p server -- --ignored` fails only on
  `the_create_wallet_endpoint_returns_addresses_to_verify_a_tron_key`, with
  `409 this xpub is already registered to another account` — the test-isolation
  defect the last entry recorded, in a file this change does not touch.
  `-p data-service -- --ignored`: 138 passed, 0 failed.
- `evm/tests/monitor_recovery` failed once in the FIRST (cold) `check.sh` run of
  this session and passed on every run after, including a clean `check.sh`. The
  last entry logged the same thing as an unexplained flake; this is the
  explanation. Those tests use wall-clock sleeps against a
  `timeout(Duration::from_secs(2))` wait for the monitor's startup event, so a
  box still finishing a full workspace compile can miss it. It is not a product
  failure and not caused by anything here. Worth knowing: `cargo test -p evm
  --test monitor_recovery` on its own will not even COMPILE — that binary needs
  `evm`'s `test-utils`, which nothing but `server`'s dev-dependency turns on, so
  it only exists under `--workspace`. Add `--features test-utils` to run it
  alone. Ten minutes went into reading that as a second, different failure.

**Next step:** F001's caller. The counter is the cheap signal and is now
trustworthy as far as it goes, which is publish failures only; nothing yet
reports either of the reconciler's two directions to an operator, and that is
the reading that would actually answer "is a watch missing".

**Blockers:** none for F002. The API 502 from two entries ago was not
re-checked.

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
