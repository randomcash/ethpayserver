# Session log

Newest first. One entry per session. The point of this file is that the next
session does not have to reconstruct what happened from the diff — write down
what you learned, not only what you changed.

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
