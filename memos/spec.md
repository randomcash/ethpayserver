# What this is

A non-custodial EVM payment processor. A merchant registers an extended public
key; the server derives a fresh receiving address per invoice from it, watches
that address on chain, and records payments as they confirm. **The server never
holds a spending key and cannot move funds.**

That guarantee is load-bearing rather than aspirational. `validate_xpub` accepts
only a base58 `xpub` and refuses an `xprv` on the version-byte prefix, so a
merchant cannot hand over a spending key even by pasting the wrong line.
Anything that would require the server to hold one is a change to what this
product *is*, not a feature.

## Shape

```
payserver-commons     shared types, auth, crypto, rates, ui-kit
       |  pinned BY REVISION in Cargo.toml
       v
this repository       API, monitor, data-service
       |  pins a published client image in ops/client-image.pin
       v
payserver-client      frontend, its own repository
```

The client never depends on a payserver — it talks to whichever one is
configured at runtime. Do not add a dependency edge from client to server.

## What works today

Measured on 2026-09-26 at `142d923`, not assumed:

- `cargo build --workspace` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo test --workspace` — **656 passed, 0 failed, 210 ignored**

The 210 need a live Postgres and Redis. They are not optional extras: CI runs
them with `--run-ignored` and they gate merges. They include the cross-tenant
isolation suite, which was verified by ablation on 2026-09-26 — the tenant guard
in `get_store_wallet` was disabled deliberately and exactly two assertions went
red, then green again when restored. That suite genuinely exercises what it
claims to.

End-to-end tests run against a real server and the **pinned** client image, so
the pin can silently fall behind and a feature can ship in the client while every
test runs against a build that predates it.

## Known weak points

These are recorded because a session that rediscovers one wastes a day.

- **A check can pass without having looked.** Several here have. Before trusting
  a green, ask what it measured and against what.
- **The client is pinned.** When the client renames a class, the selectors and
  the pin bump must move in the same commit, or each half looks broken to
  whoever touches it next.
- **Rate limits fail the end-to-end suite for the wrong reason** — the limiter
  returns 429 and logs nothing, so the server looks healthy while tests fail in
  no pattern.
- **Deploy is not merge.** A merge can succeed while its deploy fails, leaving
  the branch ahead of the running image. Verify with the health endpoint and a
  build-sha comparison, not with the merge result.

## Questions

`QUESTION:` marks something a session must not guess at.

**QUESTION: what is the near-term goal this work is measured against?** Reliable
operation on testnet, or readiness to provision a production instance? The two
imply different orderings — the first favours correctness work on what exists,
the second favours the deploy and backup paths.

**QUESTION: should an unpaid invoice ever block a merchant from deleting their
own account?** Settled on 2026-09-26 as *no*, and the guard was narrowed to
match. Recorded here because the reasoning is not obvious from the code: a
payment already *detected* still blocks deletion, via the payments-row check,
and that is the case worth refusing. "Unpaid" and "in flight" are different
things.

**QUESTION: is there an owner for the file-size backlog?** 14 of 20 open pull
requests currently fail the line-limit gate, most on production growth rather
than tests. That is a structural block on the queue rather than a lint
preference, and it needs a decision about sequencing rather than more splitting
one file at a time.

**QUESTION: what is the intended behaviour when the monitor's watch set and the
database disagree?** Nothing currently detects it. A watch present in the
monitor but absent from the database wastes a key; a watch the database expects
and the monitor does not hold means **a payment can arrive and nobody notices**.
The second is the expensive one and is not specific to any recent change.
