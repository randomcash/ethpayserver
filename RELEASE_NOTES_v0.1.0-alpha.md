# ETHPayServer v0.1.0-alpha

**First public release. Alpha, and pre-release on purpose** — see *Not ready for* below.
Stable `v0.1.0` is reserved for when mainnet is fully operational.

A self-hosted, non-custodial payment processor for Ethereum and EVM-compatible
chains, written in Rust. Merchants supply an xpub; the server derives a fresh
receive address per invoice and never holds a private key.

---

## What this release actually does

The money path is verified end to end against a live deployment, on a schedule:

```
invoice created over the API
  → real Sepolia transaction broadcast
  → detected by the chain monitor
  → invoice reaches `paid`
  → signed webhook delivered
```

That runs nightly against `testnet.random.cash` with a healthchecks.io
dead-man's switch behind it, and it is the reason this release exists. Before it,
nothing proved the deployed system could take a payment.

## Highlights

- **Multi-chain EVM support** — one codebase across Ethereum and EVM-compatible
  networks, mainnets and testnets.
- **Native and ERC-20 payments** — ETH plus whitelisted tokens.
- **Non-custodial by construction** — the server stores only an xpub, a public
  key. It derives receive addresses; it cannot spend. Losing an account never
  loses funds.
- **Real-time detection** — WebSocket block subscriptions with polling fallback,
  and reorg handling.
- **Horizontal scale** — the chain monitor is a separate binary bridged over
  Redis, so detection scales independently of the API.
- **Webhooks** — signed, with delivery logs and a testing UI.
- **Passkey, wallet, or passkey-only accounts** — sign in with WebAuthn or an
  Ethereum wallet. Passkey-only registration requires no email and no wallet:
  paste an xpub and take payments.
- **MCP server** — programmatic access for agent tooling.

## Security work in this release

- Recovery phrases are now real. Registration previously issued **every account
  the same hardcoded phrase** — the first twelve BIP-39 words, in order. Now 24
  words from `crypto.getRandomValues`, bound to the account (RCS-193).
- A valid session could be escalated to **permanent account takeover** through
  the recovery-complete endpoint, which verified no recovery secret and accepted
  a challenge minted by the add-passkey flow. Fixed (RCS-207).
- The recovery salt identifier is pinned at registration, so an account that
  gains an email later stays recoverable (RCS-201).
- RPC endpoints, database and Redis URLs are held in `SecretString` and scrubbed
  from logs, errors and telemetry.

## Not ready for

**Do not put real funds through this.**

- **Account recovery has no user interface.** The server flow exists and works;
  nothing calls it. Save your recovery phrase — it cannot be reissued — but it
  is not yet a way back into an account (RCS-205).
- **Testnet only.** Mainnet is gated on a security audit (RCS-123).
- Some write paths are not transactional and can leave orphaned records on
  partial failure (RCS-134, RCS-194).
- `/auth/recovery/start` can be driven to lock an account by anyone who knows a
  merchant's identifier (RCS-204).

## Requirements

Rust (pinned via `rust-toolchain.toml`), PostgreSQL 14+, Redis 7+, and an EVM RPC
endpoint — your own node, or a provider. See `README.md` for local setup and
`docker/` for a deployment stack.

## Feedback

Issues and questions are welcome. The most useful thing you can tell us is what
broke when you tried to take a payment.
