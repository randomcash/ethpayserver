# Recovering from an event-outbox lineage break

Status: **design only, not implemented.** Today a lineage break (stored cursor
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
   through the normal outbox, so the existing idempotent apply path credits
   the payments.
2. **A watch set that includes expired watches.** The rescan must match against
   watches that were active at any point in the window, not only the current
   in-memory set. That means loading recently-expired `watched_addresses` rows
   for the window from the database and passing them with the command.
3. **A resume position under the new epoch.** Read the stored
   `block_height` for the chain before deleting its cursor, subtract a
   reorg-depth margin, and issue the rescan from there. The cursor must not be
   deleted until the rescan command is accepted, or a failure between the two
   loses the only record of where to start.
4. **Bounded cost.** A cursor from a long-dead deployment names a height that
   could be millions of blocks back. Cap the window and halt (as today) beyond
   it.
5. **Then revisit the flag.** Once replay is the default, invert
   `EVENT_ACCEPT_LINEAGE_BREAK` into an opt-out (halt and audit by hand).

## Why this is not done in one step

Items 1 and 2 are new scanning code in `evm/`, a sensitive path that decides
whether funds are credited, and they cross the process boundary between the
server and the monitor binary. They need their own test coverage against the
mock chain source (a payment on an expired watch inside the gap is the
regression to pin) and a human review of the scan window logic. Shipping a
half-version that reads `block_height` but rescans nothing would look like
recovery while still losing payments, which is worse than halting.
