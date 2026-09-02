# Merchant API Integration Guide

ETHPayServer is a self-hosted Ethereum payment processor. This guide covers the
full merchant integration lifecycle: authentication, store setup, invoice
creation, payment monitoring, and webhook handling.

**Base URL:** `https://your-instance.example.com`

---

## Table of Contents

1. [Authentication](#1-authentication)
2. [Store Setup](#2-store-setup)
3. [Payment Methods](#3-payment-methods)
4. [Creating Invoices](#4-creating-invoices)
5. [Monitoring Payments](#5-monitoring-payments)
6. [Webhooks](#6-webhooks)
7. [WebSocket (Real-Time)](#7-websocket-real-time)
8. [Error Handling](#8-error-handling)
9. [Full Integration Example](#9-full-integration-example)

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

### Configure a Wallet (HD Wallet via xpub)

ETHPayServer derives unique payment addresses from an extended public key
(BIP-32 xpub). This means the server never holds private keys.

```bash
curl -X PUT https://your-instance.example.com/stores/{store_id}/wallet \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "xpub": "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt",
    "name": "Main Wallet"
  }'
```

Response (`200 OK`):

```json
{
  "id": "b2c3d4e5-f6a7-8901-bcde-f23456789012",
  "store_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "xpub_masked": "xpub6CU...3fDVmz",
  "derivation_index": 0,
  "name": "Main Wallet",
  "created_at": "2026-04-01T12:01:00Z"
}
```

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
    "chain_id": 1,
    "asset_symbol": "ETH",
    "decimals": 18,
    "token_address": null,
    "xpub": "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt"
  }'
```

Common configurations:

| Asset | Chain | `chain_id` | `token_address` | `decimals` |
|-------|-------|-----------|-----------------|-----------|
| ETH | Ethereum | `1` | `null` | `18` |
| ETH | Sepolia (testnet) | `11155111` | `null` | `18` |
| USDC | Ethereum | `1` | `0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48` | `6` |
| USDC | Polygon | `137` | `0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359` | `6` |
| USDT | Ethereum | `1` | `0xdAC17F958D2ee523a2206206994597C13D831ec7` | `6` |

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
      "payment_method_id": "ETH-1",
      "chain_id": 1,
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

## 5. Monitoring Payments

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
      "chain_id": 1,
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
| `invoice_expired` | Invoice expired without full payment |
| `invoice_cancelled` | Invoice was cancelled |
| `late_paid` | Payment received after invoice expiration |

### Webhook Payload

```json
{
  "event_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "event_type": "payment_confirmed",
  "timestamp": "2026-04-01T12:15:00Z",
  "invoice_id": "inv_c3d4e5f6-a7b8-9012-cdef-345678901234",
  "store_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "paid",
  "amount": "7142857142857142",
  "amount_received": "7142857142857142",
  "asset_symbol": "ETH",
  "chain_id": 1,
  "network": "mainnet",
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
| `X-Webhook-Id` | Unique event ID (use for idempotency) |

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
backoff:

| Attempt | Delay |
|---------|-------|
| 1 | 10 seconds |
| 2 | 30 seconds |
| 3 | 90 seconds |

After 3 failed attempts, the delivery is abandoned. The event is recorded in
the payment events log for later inspection.

---

## 7. WebSocket (Real-Time)

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

## 8. Error Handling

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
| `422` | Validation passed but business logic rejected |
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

Use the `X-Webhook-Id` header for webhook idempotency. Store processed event
IDs and skip duplicates. Invoice IDs are stable and can be used as idempotency
keys for status polling.

---

## 9. Full Integration Example

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
    event_id = request.headers.get("X-Webhook-Id", "")

    # Verify signature
    expected = "sha256=" + hmac.new(
        WEBHOOK_SECRET.encode(), body, hashlib.sha256
    ).hexdigest()
    if not hmac.compare_digest(expected, signature):
        return "Invalid signature", 401

    payload = json.loads(body)
    event_type = payload["event_type"]

    if event_type == "payment_confirmed":
        order_id = payload.get("metadata", {}).get("order_id")
        print(f"Payment confirmed for order {order_id}")
        # Mark order as paid in your database

    elif event_type == "invoice_expired":
        print(f"Invoice {payload['invoice_id']} expired")
        # Handle expiration (e.g., release reserved inventory)

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

## OpenAPI / Swagger

Interactive API documentation is available at `/swagger-ui` when enabled on
your instance. The OpenAPI spec is served at `/api-docs/openapi.json`.
