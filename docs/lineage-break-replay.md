# Recovering from an event-outbox lineage break

Status: **design only, not implemented.** The apply-path prerequisite (crediting
a payment to an expired watch, #310) has landed; the monitor rescan (items 1-3
below) has not, and is the remaining work. Today a lineage break (stored cursor
epoch differs from the outbox's, or the cursor is below retention) halts the
consumer with `process::exit(1)`. An operator audits the gap and may set
`EVENT_ACCEPT_LINEAGE_BREAK=true`. This note records what automatic recovery
needs, so the work can be scoped and reviewed on its own.

## What already exists

- On an accepted break, `break_lineage` calls
  `reset_chain_watch_notifications`, which sets `monitor_notified = FALSE` on
  every **active** watch of the affected chain, and deletes the stale cursor
  row. `watch_retry` then re-sends those watches to the monitor.
- `EventCursor.block_height` is committed with every envelope
  (`chain_cursors.block_height`) and never read back.

## What it cannot recover

A payment that confirmed while its watch had already expired inside the gap.
The watch is inactive, so it is not re-armed, and the monitor has no watch for
it, so nothing re-detects the transfer. Re-arming can never fix this; only
reading the chain again can.

## What replay needs

1. **A rescan capability in the monitor.** Nothing in `evm/` scans history:
   native detection reads one announced block at a time, and the RPC source
   states outright that nothing backfills. A rescan needs a new monitor
   command (`RescanFrom { chain_id, block }`), carried over the same command
   channel as `WatchAddress`, that walks `[block, tip]` for native transfers and
   for token `Transfer` logs (`LogFilter::from_block` exists) and publishes
   through the normal outbox, so the apply path sees them. Note the apply path
   does not credit inactive watches today (see below), so this alone is not
   enough.
2. **A watch set that includes expired watches.** The rescan must match against
   watches that were active at any point in the window, not only the current
   in-memory set. That means loading recently-expired `watched_addresses` rows
   for the window from the database and passing them with the command.
3. **A resume position under the new epoch.** Read the stored
   `block_height` for the chain before deleting its cursor, and start the
   rescan from the lowest block among payments not yet confirmed and pending
   watches (see the cursor-height bullet below: cursor height minus a margin
   is not a safe start). The cursor must not be
   deleted until the rescan command is accepted, or a failure between the two
   loses the only record of where to start.
4. **Bounded cost.** A cursor from a long-dead deployment names a height that
   could be millions of blocks back. Cap the window and halt (as today) beyond
   it.
5. **Then revisit the flag.** Once replay is the default, invert
   `EVENT_ACCEPT_LINEAGE_BREAK` into an opt-out (halt and audit by hand).

## Questions the review raised, checked against the source

- **Does the apply path accept a credit for a watch that has since
  expired?** Yes. `handle_payment_detected` first resolves the payment option
  through `WatchedAddressReader::get_payment_option_id`, whose queries filter
  `is_active = TRUE`, so an expired watch yields `None`. The handler then falls
  back to the invoice's own payment options, matching address, chain and token
  case-insensitively, and credits the payment with no grace window; the
  existing late-payment transition to `late_paid` flags it at confirmation. The
  fallback is covered by `payment_to_an_expired_watch_is_credited_and_settles_late`
  and `erc20_payment_to_an_expired_watch_is_credited` in
  `server/src/services/event_consumer/tests/payment_detected.rs` (landed as
  #310; both deactivate the watch first, and both go red with the fallback
  removed, the ERC-20 one asserting `credited_amount`). The address and token
  comparison is case-insensitive, which is sound only because every chain the
  server enables today uses hex EVM addresses; a chain with case-sensitive
  addresses would need an exact comparison here. What the fallback is bounded by: it only looks at the payment options
  of the invoice the event names, and only credits on an address, chain and
  token match, so it cannot attribute a transfer to another invoice; it then
  goes through the same credited-amount path as an active watch. Replay
  idempotence is expected to come from the payment upsert described below
  (`reapplying_the_same_detection_credits_once` covers the in-memory store's dedup on `(chain, tx_hash, tx_index)`, which mirrors the Postgres upsert but does not prove it; the Postgres upsert itself is covered by the ignored
  integration test
  `integration_redelivered_payment_reuses_the_original_row_id_and_does_not_duplicate_the_obligation`,
  which asserts one row and one credit). Status effects
  are those of `handle_payment_confirmed`: an expired invoice the payment
  fully covers becomes `late_paid`, a cancelled, refunded or already-paid
  invoice keeps its status, and a partial payment counts toward
  `amount_received` without changing status. Replaying rescanned transfers
  through the normal outbox therefore credits the expired-watch case, so the
  replay design does not need a separate inactive-watch lookup.
- **A replay must emit the detection before the confirmation.**
  `handle_payment_confirmed` looks the payment up by
  `(invoice_id, tx_hash, tx_index)` and, when no row exists, logs at debug and
  returns `Ok(())`. A rescan that published only a confirmation for a payment
  missed entirely would be applied, committed, and credit nothing, with no
  error. The rescan must publish `PaymentDetected` first.
- **Deduplication of re-emitted events.** Payment rows are upserted on
  `(chain_id, tx_hash, tx_index)` (`upsert_payment_row`), and `mark_confirmed`
  is a no-op once set, so re-applying a payment that was already credited does
  not double-credit. The confirmation handler looks the row up by
  `(invoice_id, tx_hash, tx_index)` instead; the two agree only while a
  transfer belongs to one invoice, which holds because a payment address maps
  to one watch. That assumption should be tested before replay relies on it.
  The ordering rule stays as in `apply_envelope`: apply, then commit the
  cursor, so a crash in between redelivers rather than loses.
- **Cursor `block_height` versus what was applied.** The cursor is committed
  only after the apply succeeds, so its height is a lower bound on what was
  applied. But a reorg-depth margin is **not** a safe rescan start. A payment
  detected at block N is confirmed at N+k; if the outbox is lost before the
  confirmation is emitted, and the cursor has moved more than the margin past
  N, a rescan from `cursor - margin` never re-reads N and the payment stays
  unconfirmed. The rescan must start from the lowest block among payments not
  yet confirmed (and pending watches), or the margin must cover confirmation
  depth plus the longest pending lifetime. Cursor height alone is not enough.
- The claims that the re-arm reset and the payment-option lookup both skip
  inactive watches are pinned by the ignored integration test
  `inactive_watch_is_neither_rearmed_nor_resolved_to_a_payment_option`
  (data-service; the re-arm half reads `monitor_notified` off the row, so dropping the `is_active` filter from the reset turns it red, and dropping it from the lookup turns the second half red).
  That test covers the data-service layer only (watch re-arm and option
  lookup skip inactive watches); the apply-path fallback that credits an
  expired watch's payment is covered separately in the event consumer tests.
  That a replay must publish detection
  before confirmation is still asserted here without a test.
- Policy, settled in #310: a late payment is credited and then flagged
  (`late_paid`) for the merchant, with no grace window. A replay over a wide
  gap applies this to every rescanned transfer to an expired invoice. Whether
  a merchant is notified of a replay-driven `late_paid` is not decided here;
  `late_paid` is the only signal, and reconciling cancelled or refunded
  invoices that gain `amount_received` is left to the merchant until the
  rescan is built and reviewed.

## Why this is not done in one step

Items 1 and 2 are new scanning code in `evm/`, a sensitive path that decides
whether funds are credited, and they cross the process boundary between the
server and the monitor binary. They need their own test coverage against the
mock chain source (a payment on an expired watch inside the gap is the
regression to pin) and a human review of the scan window logic. Shipping a
half-version that reads `block_height` but rescans nothing would look like
recovery while still losing payments, which is worse than halting.

## Status of the ticket

This note is not the ticket's deliverable. The ticket stays open: nothing reads
`block_height`, nothing rescans, and the flag is unchanged. Suggested split:
(1) apply-path credit for expired watches within a window, with a test;
(2) the monitor rescan command; (3) resume position and the flag inversion.
