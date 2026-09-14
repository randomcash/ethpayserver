# Merchant API Integration Guide

This is the one page you need to integrate with ETHPayServer. It has no
sequel — anything an integration needs (setup, invoices, webhooks, checkout,
errors, a full working example) is below, not on another page.

## What this is

ETHPayServer is a self-hosted Ethereum and EVM-chain payment processor. It is
**non-custodial**: a merchant registers an extended public key (an **xpub**,
never a private key), and every payment address is derived from it. The
server can compute addresses and watch the chain for payments to them, but it
holds no spending key for any of them.

Two consequences that follow directly from that, not incidentally:

- **The server cannot move your funds.** It never sends a transaction that
  spends merchant funds. Nothing in this API broadcasts a payout or a refund
  — see [What ETHPayServer does not do](#10-what-ethpayserver-does-not-do).
- **The server cannot recover your keys.** Only a public key is ever stored.
  If you lose the wallet that holds the matching private key, nothing here
  can get your funds back.

`POST /wallets` and `POST /stores/{id}/payment-methods` both reject a
pasted `xprv` outright — it fails the extended-key version-byte check before
it is ever stored. That is enforced in code, not just convention: see
[Add a Wallet](#add-a-wallet-hd-wallet-via-xpub).

**Base URL:** `https://your-instance.example.com`

## Machine-readable spec

The full OpenAPI 3 schema is generated from the same request/response types
the server actually serializes — it is never hand-maintained, so it cannot
drift the way prose can. On any instance with `ENABLE_SWAGGER=true` (the
server's own default; `docker/docker-compose.prod.yml` in this repo sets it
to `false`, so check with your operator if you're integrating against
someone else's deployment):

- Interactive docs: `GET /swagger-ui`
- Raw spec: `GET /api-docs/openapi.json`

Prefer the spec over this page for exact field names, optionality, and
types. This page exists for the things a schema cannot say: what order to
call things in, what a webhook delivery guarantees, what the server refuses
to do.

## Critical constraints

Read this section before writing any integration code — these are the
places a plausible-looking guess is wrong.

- **Amounts are strings, never JSON numbers, and two different kinds exist.**
  - `Invoice.amount` and `Invoice.amount_received` are decimal strings in the
    invoice's own `currency` (e.g. `"25.00"` for a `"USD"` invoice, `"0.1"`
    for an asset-denominated `"ETH"` invoice) — human-readable, always,
    regardless of whether `currency` is fiat or a crypto asset symbol. Never
    send a pre-converted (smallest-unit) value as the request `amount`; a
    same-asset invoice runs it through `convert_human_to_smallest_unit`
    server-side (`server/src/api/invoices/crud.rs`). The upstream doc comment
    on `CreateInvoiceRequest.amount` in `payserver-commons` was
    self-contradictory on this exact point; it is fixed at the source
    (`api-types/src/invoice.rs`), and the generated spec will carry the
    correct description once this repo's commons pin moves to that revision.
    Until then, trust this page and the linked server code over the spec's
    field description for this one field.
  - `PaymentOption.amount`, `Payment.amount`, and refund/payout `amount`
    fields are integer strings in the asset's **smallest unit** (wei for
    ETH, the ERC20's own base unit for a token) — divide by `10^decimals` to
    get a display value, using integer or bignum arithmetic. Never parse
    either kind as a float: float rounding on a value that settles a payment
    is a bug, not a rounding error.
- **The server never sends funds.** `POST /invoices/{id}/refund` and
  `POST /stores/{id}/payouts` create ledger records only. See
  [What ETHPayServer does not do](#10-what-ethpayserver-does-not-do).
- **Chain IDs are [CAIP-2](https://standards.chainagnostic.org/CAIPs/caip-2)
  strings** (`"eip155:1"`), not bare integers. See
  [Chain identifiers](#chain-identifiers).
- **Webhook delivery is at-least-once.** You will sometimes receive the same
  logical event twice. Dedupe on `idempotency_key`, not `event_id`. See
  [Webhooks](#6-webhooks).

---

## Table of Contents

1. [Authentication](#1-authentication)
2. [Store Setup](#2-store-setup)
3. [Payment Methods](#3-payment-methods)
4. [Creating Invoices](#4-creating-invoices)
5. [Retrying Safely: the `Idempotency-Key` Header](#5-retrying-safely-the-idempotency-key-header)
6. [Webhooks](#6-webhooks)
7. [Monitoring Payments](#7-monitoring-payments)
8. [WebSocket (Real-Time)](#8-websocket-real-time)
9. [Checkout: What the Customer Sees](#9-checkout-what-the-customer-sees)
10. [What ETHPayServer Does Not Do](#10-what-ethpayserver-does-not-do)
11. [Error Handling](#11-error-handling)
12. [Full Integration Example](#12-full-integration-example)

---

## 1. Authentication

All API requests require a Bearer token in the `Authorization` header.

```
Authorization: Bearer <token>
```

### Creating an API Key

API keys are the recommended authentication method for server-to-server
integrations. Keys are shown in plaintext only once at creation time.

```bash
curl -X POST https://your-instance.example.com/users/api-keys \
  -H "Authorization: Bearer <session_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production Integration",
    "expires_at": "2027-01-01T00:00:00Z"
  }'
```

Response (`201 Created`):

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Production Integration",
  "key_prefix": "ak_****f6a8",
  "is_active": true,
  "created_at": "2026-04-01T12:00:00Z",
  "expires_at": "2027-01-01T00:00:00Z",
  "key": "ak_a1b2_c3d4e5f6789012345678901234567890abcdef"
}
```

> **Important:** Save the `key` field immediately. It cannot be retrieved again.

Use the key as a Bearer token for all subsequent requests:

```bash
curl -H "Authorization: Bearer ak_a1b2_c3d4e5f6789012345678901234567890abcdef" \
  https://your-instance.example.com/stores
```

### Listing API Keys

```bash
curl https://your-instance.example.com/users/api-keys \
  -H "Authorization: Bearer <token>"
```

### Revoking an API Key

```bash
curl -X DELETE https://your-instance.example.com/users/api-keys/{id} \
  -H "Authorization: Bearer <token>"
```

---

## 2. Store Setup

A store represents a merchant entity. Each store has its own wallet, payment
methods, webhook configuration, and invoices.

### Create a Store

```bash
curl -X POST https://your-instance.example.com/stores \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Shop",
    "website": "https://myshop.example.com"
  }'
```

Response (`201 Created`):

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "My Shop",
  "website": "https://myshop.example.com",
  "owner_id": "11111111-2222-3333-4444-555555555555",
  "archived": false,
  "created_at": "2026-04-01T12:00:00Z"
}
```

### Add a Wallet (HD Wallet via xpub)

ETHPayServer derives unique payment addresses from an extended public key
(BIP-32 xpub). This means the server never holds private keys.

Wallets belong to the account, not to a store. Every store uses the account's
primary wallet unless it is pinned to a different one.

```bash
curl -X POST https://your-instance.example.com/wallets \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "xpub": "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt",
    "name": "Main Wallet"
  }'
```

Response (`201 Created`):

```json
{
  "id": "b2c3d4e5-f6a7-8901-bcde-f23456789012",
  "user_id": "11111111-2222-3333-4444-555555555555",
  "xpub_masked": "xpub6CU...3fDVmz",
  "derivation_index": 0,
  "name": "Main Wallet",
  "is_primary": true,
  "created_at": "2026-04-01T12:01:00Z"
}
```

If `xpub` fails the extended-key version-byte check — including every
`xprv`, which is a spending key and uses a different version byte on
purpose — this returns `400 Bad Request` and nothing is stored. There is no
way to register a spending key through this or any other endpoint; that is
what makes the non-custodial guarantee true rather than aspirational.

The first wallet you add becomes the primary. `PATCH /wallets/{wallet_id}` with
`{"is_primary": true}` moves it later.

Adding an xpub you already hold returns the wallet you already have rather than
a second one. A key has exactly one derivation counter: a second counter on the
same key would hand the same address to two different customers.

To give one store its own wallet:

```bash
curl -X PUT https://your-instance.example.com/stores/{store_id}/wallet \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"wallet_id": "b2c3d4e5-f6a7-8901-bcde-f23456789012"}'
```

`DELETE /stores/{store_id}/wallet` removes that pin so the store follows the
primary again. It does not delete the wallet, and no derivation counter is
reset.

Both calls change where money actually goes: the store's payment methods are
released from whatever key they were configured with and follow the store from
then on, so the next invoice is paid to an address derived from the wallet you
named. Addresses already issued keep working - in-flight invoices still resolve
on them.

An xpub can belong to only one account. Registering one another merchant has
already added returns `409 Conflict`, because two accounts deriving from one
key would hand the same addresses to both their customers.

The xpub must be at the BIP-44 account level (`m/44'/60'/0'`). Payment
addresses are derived at `m/44'/60'/0'/0/{index}`.

---

## 3. Payment Methods

Payment methods define which chains and tokens a store accepts. Each method uses
the store's xpub for address derivation.

### Enable a Payment Method

```bash
curl -X POST https://your-instance.example.com/stores/{store_id}/payment-methods \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "chain_id": "eip155:1",
    "asset_symbol": "ETH",
    "decimals": 18,
    "token_address": null,
    "xpub": "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt"
  }'
```

Common configurations:

| Asset | Chain | `chain_id` | `token_address` | `decimals` |
|-------|-------|-----------|-----------------|-----------|
| ETH | Ethereum | `eip155:1` | `null` | `18` |
| ETH | Sepolia (testnet) | `eip155:11155111` | `null` | `18` |
| USDC | Ethereum | `eip155:1` | `0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48` | `6` |
| USDC | Polygon | `eip155:137` | `0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359` | `6` |
| USDT | Ethereum | `eip155:1` | `0xdAC17F958D2ee523a2206206994597C13D831ec7` | `6` |

`xpub` here is optional: omit it to use whatever wallet the store already
resolves to (its pinned wallet, or the account's primary). Pass it only to
set a different key for this specific method than the store otherwise uses.
When given, it is checked the same way as in
[Add a Wallet](#add-a-wallet-hd-wallet-via-xpub) — an `xprv` is refused.

### List Payment Methods

```bash
curl https://your-instance.example.com/stores/{store_id}/payment-methods \
  -H "Authorization: Bearer <token>"
```

### Disable a Payment Method

```bash
curl -X PUT https://your-instance.example.com/stores/{store_id}/payment-methods/{method_id} \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}'
```

---

## 4. Creating Invoices

An invoice represents a payment request. When created, ETHPayServer derives a
unique address for each enabled payment method and begins monitoring the
blockchain for incoming payments.

### Create an Invoice

```bash
curl -X POST https://your-instance.example.com/invoices \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "store_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "currency": "USD",
    "amount": "25.00",
    "expiration_seconds": 900,
    "metadata": {"order_id": "ORD-12345"},
    "webhook_url": "https://myshop.example.com/webhooks/payments",
    "redirect_url": "https://myshop.example.com/order/ORD-12345/complete"
  }'
```

Response (`201 Created`):

```json
{
  "id": "inv_c3d4e5f6-a7b8-9012-cdef-345678901234",
  "currency": "USD",
  "status": "pending",
  "amount": "25.00",
  "amount_received": "0",
  "created_at": "2026-04-01T12:05:00Z",
  "expires_at": "2026-04-01T12:20:00Z",
  "metadata": {"order_id": "ORD-12345"},
  "payment_options": [
    {
      "id": "po_d4e5f6a7-b8c9-0123-def4-567890123456",
      "payment_method_id": "ETH@eip155:1",
      "chain_id": "eip155:1",
      "asset_symbol": "ETH",
      "token_address": null,
      "decimals": 18,
      "payment_address": "0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18",
      "amount": "7142857142857142",
      "rate": "0.00035",
      "is_active": true
    }
  ]
}
```

`amount` on the invoice itself is `"25.00"` — a decimal string in `USD`, the
invoice's `currency`. `payment_options[].amount` is `"7142857142857142"` — an
integer string of wei, the smallest unit of `ETH`. These are two different
kinds of amount on the same object; see
[Critical constraints](#critical-constraints).

Send the customer to `payment_options[].payment_address` for the amount and
asset they choose, or to the hosted checkout page — see
[Checkout: What the Customer Sees](#9-checkout-what-the-customer-sees).

**Parameters:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `store_id` | UUID | Yes | Store to create invoice for |
| `currency` | String | Yes | Invoice currency (`USD`, `EUR`, `ETH`, etc.) |
| `amount` | String | Yes | Amount in invoice currency (e.g., `"25.00"`) |
| `expiration_seconds` | Integer | No | Seconds until expiry (default: 900) |
| `metadata` | JSON | No | Custom data attached to the invoice |
| `webhook_url` | String | No | Override store webhook for this invoice |
| `redirect_url` | String | No | URL to redirect customer after payment |

### Invoice Statuses

| Status | Description |
|--------|-------------|
| `pending` | Awaiting payment |
| `processing` | Payment detected, awaiting confirmations |
| `partially_paid` | Some payments confirmed, amount not yet fulfilled |
| `paid` | Full amount confirmed |
| `expired` | Expiration reached without full payment |
| `late_paid` | Payment received after expiration |
| `cancelled` | Cancelled by store admin |

### List Invoices

```bash
curl "https://your-instance.example.com/invoices?store_id={store_id}&status=pending&limit=20&offset=0" \
  -H "Authorization: Bearer <token>"
```

Response:

```json
{
  "total": 42,
  "invoices": [...]
}
```

### Cancel an Invoice

Only `pending`, `processing`, or `partially_paid` invoices can be cancelled.

```bash
curl -X POST https://your-instance.example.com/invoices/{invoice_id}/cancel \
  -H "Authorization: Bearer <token>"
```

---

## 5. Retrying Safely: the `Idempotency-Key` Header

Network errors and timeouts can make it impossible to know whether a request
succeeded. This is a *client-supplied* key for safe retries of a mutation —
distinct from the *server-supplied* `idempotency_key` on webhook deliveries
covered in [Webhooks](#6-webhooks); the two solve different problems and
neither substitutes for the other.

### How it works

Include an `Idempotency-Key` header on the request:

```
POST /invoices HTTP/1.1
Authorization: Bearer <token>
Idempotency-Key: <unique-key>
Content-Type: application/json

{"store_id": "...", "currency": "USD", "amount": "100.00"}
```

The key can be any ASCII string up to 255 characters. A UUID or ULID is
recommended.

### Behaviour

| Scenario | Response |
|----------|----------|
| First request with this key | Normal response (e.g. `201 Created`) |
| Retry with same key + same body | Cached response replayed with `Idempotency-Replayed: true` header |
| Retry with same key + different body | `409 Conflict` `{"error": "idempotency_key_reuse"}` |
| Second request while first is still in-flight | `425 Too Early` `{"error": "idempotency_in_progress"}` |
| Invalid key (empty, >255 chars, non-ASCII) | `400 Bad Request` `{"error": "idempotency_key_invalid"}` |

### Details

- Keys are scoped per authentication credential (session or API key).
- Only **successful** (2xx) responses are cached. Server errors can be retried
  with the same key.
- Cached responses expire after **24 hours** (configurable via
  `IDEMPOTENCY_TTL_SECS` environment variable).
- The `Idempotency-Key` header is optional. Requests without it are processed
  normally with no caching.
- Supported on every POST under `/invoices`: create (`POST /invoices`),
  cancel (`POST /invoices/{id}/cancel`), and refund
  (`POST /invoices/{id}/refund` — see
  [What ETHPayServer does not do](#10-what-ethpayserver-does-not-do) for what
  that endpoint actually does).
- Maximum request body size is 1 MiB; larger POSTs with an idempotency
  key return `413 Payload Too Large`.

---

## 6. Webhooks

Webhooks notify your server of payment events in real time. Configure a webhook
URL on your store, and ETHPayServer will POST event payloads to it.

### Configure a Webhook

```bash
curl -X PUT https://your-instance.example.com/stores/{store_id}/webhook \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "webhook_url": "https://myshop.example.com/webhooks/payments",
    "enabled": true
  }'
```

The response includes a `webhook_secret` for signature verification. Save it
securely -- it is regenerated on each update.

### Webhook Events

| Event | Description |
|-------|-------------|
| `payment_detected` | Payment seen on-chain, awaiting confirmations |
| `payment_confirmed` | Payment confirmed (reached confirmation threshold) |
| `payment_reorged` | Previously reported payments were retracted by a chain reorganization |
| `invoice_expired` | Invoice expired without full payment |
| `invoice_cancelled` | Invoice was cancelled |
| `late_paid` | Payment received after invoice expiration |

This table is the whole vocabulary: every event listed is emitted by some code
path, and nothing outside it is ever sent. New event types may be added, so
ignore an `event_type` you do not recognise rather than failing the delivery.

#### Retractions

`payment_reorged` is a **retraction**, not a restatement. A chain
reorganization can remove a transaction you were already told about by
`payment_detected` or `payment_confirmed`. When that happens:

- `status` is the status the invoice was reverted **to** (`processing` if other
  valid payments remain, otherwise `pending`).
- `amount_received` is the amount that survives the reorg.
- `retracted_payments` lists every transaction that no longer exists on the
  canonical chain.

If you credited any of those transactions, reverse the credit. There is no
`payment` object on this event; the retracted transactions are in
`retracted_payments`.

```json
{
  "version": 1,
  "event_id": "8a1f0b0c-6a2e-4c8b-9d5a-2f3e4b5c6d7e",
  "idempotency_key": "evt_9f2c...",
  "event_type": "payment_reorged",
  "timestamp": "2026-04-01T12:31:00Z",
  "invoice_id": "inv_c3d4e5f6-a7b8-9012-cdef-345678901234",
  "store_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "pending",
  "amount": "7142857142857142",
  "amount_received": "0",
  "asset_symbol": "ETH",
  "chain_id": "eip155:1",
  "retracted_payments": [
    {
      "tx_hash": "0xabc123def456...",
      "from_address": "0x1234567890abcdef...",
      "block_number": 19500000,
      "confirmed": false
    }
  ]
}
```

### Webhook Payload

```json
{
  "version": 1,
  "event_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "idempotency_key": "evt_3b7a1c...",
  "event_type": "payment_confirmed",
  "timestamp": "2026-04-01T12:15:00Z",
  "invoice_id": "inv_c3d4e5f6-a7b8-9012-cdef-345678901234",
  "store_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "paid",
  "amount": "7142857142857142",
  "amount_received": "7142857142857142",
  "asset_symbol": "ETH",
  "chain_id": "eip155:1",
  "payment": {
    "tx_hash": "0xabc123def456...",
    "from_address": "0x1234567890abcdef...",
    "block_number": 19500000,
    "confirmed": true
  }
}
```

### Webhook Headers

| Header | Description |
|--------|-------------|
| `Content-Type` | `application/json` |
| `X-Webhook-Signature` | `sha256=<hex>` HMAC-SHA256 of the JSON body |
| `X-Webhook-Event` | Event type (e.g., `payment_confirmed`) |
| `X-Webhook-Id` | Identifier for this delivery |
| `X-Webhook-Idempotency-Key` | Stable key for the logical event (dedupe on this) |

### Payload Versioning

Every payload carries a `version`. The contract is **additive-only**: new
fields and new event types may appear at any time without a version bump, so
ignore what you do not recognise. Removing or renaming a field, changing its
type, or changing the meaning of an existing value requires a version bump.

Optional fields are omitted rather than sent as `null` or as a placeholder;
absent means "does not apply to this event".

### Verifying Signatures

Compute an HMAC-SHA256 of the raw request body using your webhook secret, then
compare with the `X-Webhook-Signature` header.

**Python:**

```python
import hmac
import hashlib

def verify_webhook(body: bytes, signature: str, secret: str) -> bool:
    expected = "sha256=" + hmac.new(
        secret.encode(), body, hashlib.sha256
    ).hexdigest()
    return hmac.compare_digest(expected, signature)
```

**JavaScript (Node.js):**

```javascript
const crypto = require('crypto');

function verifyWebhook(body, signature, secret) {
  const expected = 'sha256=' +
    crypto.createHmac('sha256', secret).update(body).digest('hex');
  return crypto.timingSafeEqual(
    Buffer.from(expected), Buffer.from(signature)
  );
}
```

### Retry Policy

Failed deliveries (non-2xx response or timeout) are retried with exponential
backoff, up to a fixed number of attempts, then the delivery is dropped and
recorded in the payment events log for later inspection. The exact delays and
attempt count are `WebhookJob::RETRY_DELAYS_SECS` and `max_attempts` in
`server/src/services/webhook/job.rs` — read those rather than this paragraph
if you need to reason about worst-case delivery latency, since this prose
copy is exactly the kind of restatement that drifts the moment the code
changes.

### Webhook Delivery Idempotency

**Webhook delivery is at-least-once**, not exactly-once. A delivery is
retried on any non-2xx response or transport error, and a handler that
re-runs after a restart can emit the same logical event again. You will
sometimes receive an event twice. There is no ordering guarantee between
events for different invoices.

Dedupe on `idempotency_key` (also sent as the `X-Webhook-Idempotency-Key`
header, so you can drop a repeat before parsing the body). It is derived from
what happened -- the event type, the invoice, and the specific transition --
so the same logical event always carries the same key. Record the keys you
have processed and skip repeats.

Do **not** dedupe on `event_id`: it identifies one queued delivery, and a
re-emission of the same logical event carries a fresh one.

This is a different mechanism from the `Idempotency-Key` *request* header in
[section 5](#5-retrying-safely-the-idempotency-key-header) — that one is a
key you send when creating an invoice; this one is a key the server sends you
on a webhook delivery. Don't conflate them.

---

## 7. Monitoring Payments

### Poll Invoice Status

```bash
curl https://your-instance.example.com/invoices/{invoice_id}/status \
  -H "Authorization: Bearer <token>"
```

Response:

```json
{
  "id": "inv_c3d4e5f6-a7b8-9012-cdef-345678901234",
  "status": "processing",
  "amount": "25.00",
  "amount_received": "25.00",
  "currency": "USD",
  "expires_at": "2026-04-01T12:20:00Z",
  "payment_count": 1,
  "confirmed_count": 0,
  "is_paid": false,
  "is_expired": false,
  "payment_options": [...],
  "payments": [
    {
      "id": "pay_e5f6a7b8-c9d0-1234-ef56-789012345678",
      "chain_id": "eip155:1",
      "invoice_id": "inv_c3d4e5f6-a7b8-9012-cdef-345678901234",
      "tx_hash": "0xabc123def456...",
      "amount": "7142857142857142",
      "asset_symbol": "ETH",
      "token_address": null,
      "block_number": 19500000,
      "from_address": "0x1234567890abcdef...",
      "detected_at": "2026-04-01T12:08:30Z",
      "confirmed_at": null,
      "reorged": false
    }
  ]
}
```

Webhooks are strictly preferred over polling for anything that runs on a
schedule: polling costs a request per check and still lags real-time,
whereas a webhook fires the moment the state changes. Use polling for a
one-off status check, not as the primary mechanism.

### List All Payments

```bash
curl "https://your-instance.example.com/payments?store_id={store_id}&status=confirmed&limit=50" \
  -H "Authorization: Bearer <token>"
```

### Get a Single Payment

```bash
curl https://your-instance.example.com/payments/{payment_id} \
  -H "Authorization: Bearer <token>"
```

---

## 8. WebSocket (Real-Time)

For real-time updates without polling, connect via WebSocket.

### Connecting

```
ws://your-instance.example.com/ws?token=<session_token>
```

On successful connection, the server sends:

```json
{"type": "connected"}
```

### Message Types

**Invoice status change:**

```json
{
  "type": "invoice_status",
  "invoice_id": "inv_c3d4e5f6...",
  "status": "paid"
}
```

**Payment update:**

```json
{
  "type": "payment_update",
  "payment_id": "pay_e5f6a7b8...",
  "invoice_id": "inv_c3d4e5f6...",
  "status": "confirmed",
  "amount": "7142857142857142"
}
```

**Keep-alive ping:**

```json
{"type": "ping"}
```

### JavaScript Example

```javascript
const ws = new WebSocket(
  `wss://your-instance.example.com/ws?token=${sessionToken}`
);

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  switch (data.type) {
    case 'connected':
      console.log('Connected to payment updates');
      break;
    case 'invoice_status':
      console.log(`Invoice ${data.invoice_id}: ${data.status}`);
      break;
    case 'payment_update':
      console.log(`Payment ${data.payment_id}: ${data.status}`);
      break;
  }
};
```

---

## 9. Checkout: What the Customer Sees

You are not required to build a payment UI. The companion
[payserver-client](https://github.com/randomcash/payserver-client) ships a
hosted checkout page at `/checkout/{invoice_id}`; redirect your customer
there. Its reference nginx setup proxies that page's data calls to the API's
own `GET /api/checkout/{invoice_id}`, which strips to `GET /checkout/{invoice_id}`
on the bare ethpayserver server — adjust the prefix to match how your
deployment fronts the two services.

This endpoint is public and needs no authentication — an invoice ID is
enough to view it, and it exposes payment-relevant fields only (no store
internals, no merchant account data). Call it directly if you'd rather build
your own checkout UI instead of using the hosted page.

On this page the customer sees:

- **A chain/asset selector**, when the invoice has more than one active
  payment option (e.g. pay in ETH or in USDC) — one tab per option.
- **A QR code**, encoded as an [EIP-681](https://eips.ethereum.org/EIPS/eip-681)
  payment request URI rather than a bare address, so a wallet that scans it
  prefills the chain, the address, and the exact amount.
- **The payment address and amount**, in the selected asset, with a
  copy-to-clipboard control for wallets that don't scan.
- **A countdown timer** to the invoice's `expires_at`.
- Live status updates over the same WebSocket described in
  [section 8](#8-websocket-real-time) — the page updates itself when a
  payment is detected and confirmed, with no reload.

If you build your own checkout UI instead of using the hosted page, the
`payment_options` array from [Creating Invoices](#4-creating-invoices) or
[Monitoring Payments](#7-monitoring-payments) has everything you need to
reproduce it: address, amount, asset, chain.

---

## 10. What ETHPayServer Does Not Do

Restated plainly, because a model asked to fill a gap will otherwise invent
a plausible-sounding answer:

- **No custody, ever.** The server never holds a spending key for any
  merchant funds. See [What this is](#what-this-is).
- **No refunds are sent.** `POST /invoices/{invoice_id}/refund` validates the
  request (invoice is paid, amount doesn't exceed what's left after prior
  refunds, a destination address is known) and writes a `Refund` record with
  status `Pending`. **That is all it does.** No transaction is signed or
  broadcast — the server holds no spending key to sign one with. The record
  exists so you have somewhere to track a refund you send yourself, from
  whatever wallet actually holds the funds. Refunds are the merchant's job.
- **No payouts are sent.** `POST /stores/{store_id}/payouts` is the same
  shape: it validates which confirmed, unclaimed invoice payments the
  request covers and writes a `Payout` record with status `Pending`. It does
  not sweep funds anywhere. Moving the funds it describes is, again, on you.
- **No automatic settlement or sweeping.** There is no background process
  that moves confirmed payments out of payment addresses. Funds sit at the
  addresses they were paid to until you spend them with the key that
  controls that xpub.

If your integration needs refunds or payouts to actually move money,
build that against your own wallet infrastructure — this API's refund and
payout endpoints are bookkeeping for that process, not a substitute for it.

---

## 11. Error Handling

### HTTP Status Codes

| Code | Meaning |
|------|---------|
| `200` | Success |
| `201` | Resource created |
| `204` | Success (no body) |
| `400` | Bad request (invalid parameters) |
| `401` | Missing or invalid authentication |
| `403` | Insufficient permissions |
| `404` | Resource not found |
| `409` | Conflict (e.g. duplicate xpub, idempotency key reuse) |
| `413` | Request body too large |
| `422` | Well-formed JSON body, but a field fails validation while deserializing — e.g. a `chain_id` that isn't valid CAIP-2 in a `POST /stores/{id}/payment-methods` body. This is axum's default response for that class of error. A malformed `chain_id` in a URL path segment (e.g. `GET /invoices/by-tx/{chain_id}/{tx_hash}`) is parsed explicitly by the handler instead and returns `400`. |
| `425` | Idempotent request already in flight |
| `500` | Internal server error |

### Common Error Patterns

**Invalid authentication:**

```
HTTP/1.1 401 Unauthorized
```

**Missing permissions:**

```
HTTP/1.1 403 Forbidden
```

**Invalid invoice amount:**

```
HTTP/1.1 400 Bad Request
"Invalid amount"
```

**No payment methods configured:**

```
HTTP/1.1 400 Bad Request
"Store has no active payment methods"
```

### Idempotency

See [section 5](#5-retrying-safely-the-idempotency-key-header) for safe
request retries and [Webhook Delivery Idempotency](#webhook-delivery-idempotency)
for deduping webhook deliveries — they are separate mechanisms, covered in
full where they're most relevant rather than restated here.

Invoice IDs are themselves stable and can be used as your own idempotency
keys for status polling.

---

## 12. Full Integration Example

This example walks through a complete payment flow in Python.

```python
import requests
import hmac
import hashlib
import json
from flask import Flask, request

API_BASE = "https://your-instance.example.com"
API_KEY = "ak_a1b2_c3d4e5f6..."
WEBHOOK_SECRET = "whsec_..."
STORE_ID = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"

headers = {
    "Authorization": f"Bearer {API_KEY}",
    "Content-Type": "application/json",
}

# --- Step 1: Create an invoice ---
invoice = requests.post(f"{API_BASE}/invoices", headers=headers, json={
    "store_id": STORE_ID,
    "currency": "USD",
    "amount": "25.00",
    "expiration_seconds": 900,
    "metadata": {"order_id": "ORD-12345"},
}).json()

print(f"Invoice ID: {invoice['id']}")
print(f"Status: {invoice['status']}")
for opt in invoice["payment_options"]:
    print(f"  Pay {opt['amount']} {opt['asset_symbol']} to {opt['payment_address']}")

# --- Step 2: Poll for status (alternative to webhooks) ---
status = requests.get(
    f"{API_BASE}/invoices/{invoice['id']}/status",
    headers=headers,
).json()

print(f"Paid: {status['is_paid']}, Expired: {status['is_expired']}")

# --- Step 3: Handle webhooks ---
app = Flask(__name__)

@app.route("/webhooks/payments", methods=["POST"])
def handle_webhook():
    body = request.get_data()
    signature = request.headers.get("X-Webhook-Signature", "")
    idempotency_key = request.headers.get("X-Webhook-Idempotency-Key", "")

    # Verify signature
    expected = "sha256=" + hmac.new(
        WEBHOOK_SECRET.encode(), body, hashlib.sha256
    ).hexdigest()
    if not hmac.compare_digest(expected, signature):
        return "Invalid signature", 401

    # Dedupe on idempotency_key before doing anything else (see section 6) --
    # `already_processed` and `mark_processed` are your own storage, not part
    # of this API.
    if already_processed(idempotency_key):
        return "OK", 200

    payload = json.loads(body)
    event_type = payload["event_type"]

    if event_type == "payment_confirmed":
        order_id = payload.get("metadata", {}).get("order_id")
        print(f"Payment confirmed for order {order_id}")
        # Mark order as paid in your database

    elif event_type == "invoice_expired":
        print(f"Invoice {payload['invoice_id']} expired")
        # Handle expiration (e.g., release reserved inventory)

    mark_processed(idempotency_key)
    return "OK", 200
```

---

## Permissions Reference

Store members have role-based permissions:

| Permission | Description |
|-----------|-------------|
| `cancreateinvoice` | Create invoices for the store |
| `canviewstoresettings` | View wallet and webhook configuration |
| `canmodifystoresettings` | Modify wallet, webhook, and payment methods |
| `canviewstoreusers` | List store members |
| `canmodifystoreusers` | Add, update, or remove store members |

---

## Chain identifiers

Every field naming a chain is a [CAIP-2](https://standards.chainagnostic.org/CAIPs/caip-2)
identifier — a string like `eip155:1`, not a number. That includes
`chain_id` on invoices, payments, payment options and payment methods, and
`default_chain_id` on store settings.

**Breaking changes to be aware of:**

| endpoint | before | now |
| -- | -- | -- |
| `PATCH /stores/{id}/settings` | `{"default_chain_id": 1}` | `{"default_chain_id": "eip155:1"}` |
| `GET /stores/{id}/settings` | `"default_chain_id": 1` | `"default_chain_id": "eip155:1"` |
| `POST /stores/{id}/payment-methods` | `{"chain_id": 1}` | `{"chain_id": "eip155:1"}` |
| webhook payloads | `"chain_id": 1` | `"chain_id": "eip155:1"`, absent on invoice-level events |
| `payment_method_id` | `ETH-1` | `ETH@eip155:1` |

A malformed identifier is rejected by the request parser as **422**, not 400 —
the shape is wrong, not the value.

Only `eip155` and `tron` references are numbers. Solana, Monero and Bitcoin use
truncated genesis hashes, so nothing may infer a chain's name from its
identifier; `GET /health/chains` and the chain configuration carry the display
names.
