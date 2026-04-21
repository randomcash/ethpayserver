# ETHPayServer API Reference

## Retries and Idempotency

Network errors and timeouts can make it impossible to know whether a request succeeded.
ETHPayServer supports idempotency keys so you can safely retry mutations without
creating duplicate resources.

### How it works

Include an `Idempotency-Key` header on `POST /invoices`:

```
POST /invoices HTTP/1.1
Authorization: Bearer <session_id>
Idempotency-Key: <unique-key>
Content-Type: application/json

{"store_id": "...", "currency": "USD", "amount": "100.00"}
```

The key can be any ASCII string up to 255 characters. A UUID or ULID is recommended.

### Behaviour

| Scenario | Response |
|----------|----------|
| First request with this key | Normal response (e.g. `201 Created`) |
| Retry with same key + same body | Cached response replayed with `Idempotency-Replayed: true` header |
| Retry with same key + different body | `409 Conflict` `{"error": "idempotency_key_reuse"}` |
| Second request while first is still in-flight | `425 Too Early` `{"error": "idempotency_in_progress"}` |
| Invalid key (empty, >255 chars, non-ASCII) | `400 Bad Request` `{"error": "idempotency_key_invalid"}` |

### Details

- Keys are scoped per authentication credential (session).
- Only **successful** (2xx) responses are cached. Server errors can be retried
  with the same key.
- Cached responses expire after **24 hours** (configurable via
  `IDEMPOTENCY_TTL_SECS` environment variable).
- The `Idempotency-Key` header is optional. Requests without it are processed
  normally with no caching.
- Supported on every POST under `/invoices`: create (`POST /invoices`),
  cancel (`POST /invoices/{id}/cancel`), and refund
  (`POST /invoices/{id}/refund`). Future endpoints (`/payouts`, …) will
  adopt the same mechanism.
- Maximum request body size is 1 MiB; larger POSTs with an idempotency
  key return `413 Payload Too Large`.
