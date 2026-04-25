# Sizing & Load-Testing Baseline

This document describes the load-testing suite for ethpayserver and provides
baseline numbers for capacity planning on a small self-hosted deployment.

## Reference hardware

All baseline numbers are collected on a **2 vCPU / 4 GiB RAM** VPS with
SSD-backed Postgres and Redis on the same box (Hetzner CX22, DigitalOcean
Basic, or equivalent).

## Scenarios

### 1. Invoice creation (write-heavy)

| Metric    | Baseline target |
|-----------|-----------------|
| Sustained | 50 RPS          |
| p95       | < 200 ms        |

Binary: `loadtest --scenarios "InvoiceCreate"`

Exercises `POST /invoices` with random amounts. Stresses the invoice creation
path: UUID generation, DB insert, payment-option derivation, exchange-rate
lookup.

### 2. WebSocket connections

| Metric         | Baseline target                  |
|----------------|----------------------------------|
| Concurrent     | 500 clients                      |
| Disconnect rate| < 0.1% over 5 minutes           |

Binary: `loadtest-ws`

Establishes N WebSocket connections with gradual ramp-up and holds them for a
configurable duration. Measures unexpected disconnect rate and message
throughput.

### 3. Webhook delivery (queue throughput)

| Metric    | Baseline target                         |
|-----------|-----------------------------------------|
| Sustained | 100 deliveries/min                      |
| Queue     | No growing depth during sustained load  |

Binary: `loadtest --scenarios "WebhookBurst"`

Creates invoices with `webhook_url` attached, stressing the webhook queue
insertion path. Full delivery throughput should be validated with a webhook
receiver running alongside the test.

### 4. Postgres pool saturation (read-heavy)

| Metric    | Baseline target                       |
|-----------|---------------------------------------|
| Concurrent| 20 queries                            |
| p95 wait  | < 50 ms connection acquisition        |

Binary: `loadtest --scenarios "InvoiceList"`

Fires concurrent `GET /invoices` queries with pagination, stressing the
Postgres connection pool and query planner.

## Running the suite

### Prerequisites

- A running ethpayserver instance (local or testnet)
- An API key (`ak_…`) with access to a test store
- Redis and Postgres backing the instance

### HTTP scenarios (Goose)

```sh
export LOADTEST_API_KEY="ak_test_..."
export LOADTEST_STORE_ID="<store-uuid>"

# All scenarios, 20 users, 60-second run:
cargo run -p loadtest --bin loadtest -- \
  --host http://localhost:3000 --users 20 --run-time 60s

# Single scenario:
cargo run -p loadtest --bin loadtest -- \
  --host http://localhost:3000 --users 50 --run-time 120s \
  --scenarios "InvoiceCreate"
```

### WebSocket scenario

```sh
export LOADTEST_WS_URL="ws://localhost:3000"
export LOADTEST_WS_CLIENTS=500
export LOADTEST_WS_DURATION_SECS=300

cargo run -p loadtest --bin loadtest-ws
```

## Interpreting regressions

The CI scheduled job runs the suite weekly against testnet. A regression is
flagged when any metric degrades by more than **25%** from its recorded
baseline. The regression gate is initially disabled (`allow_failure: true`)
until the first stable baseline run produces reference numbers.

Once a baseline is established:
1. Update the target numbers in this document
2. Enable the regression gate in `.gitlab-ci.yml` by removing `allow_failure`

## Environment variables reference

| Variable                        | Required | Default                  | Description                          |
|---------------------------------|----------|--------------------------|--------------------------------------|
| `LOADTEST_API_KEY`              | yes      | —                        | API key for authentication           |
| `LOADTEST_STORE_ID`             | yes      | —                        | Store UUID                           |
| `LOADTEST_WEBHOOK_RECEIVER_URL` | no       | http://localhost:9999/webhook | Webhook receiver endpoint       |
| `LOADTEST_WS_URL`               | no       | ws://localhost:3000      | WebSocket URL                        |
| `LOADTEST_WS_CLIENTS`           | no       | 500                      | Number of WS clients                 |
| `LOADTEST_WS_DURATION_SECS`     | no       | 300                      | WS hold duration in seconds          |
| `LOADTEST_WS_RAMP_SECS`         | no       | 30                       | WS client ramp-up period             |
