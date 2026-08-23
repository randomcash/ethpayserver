/**
 * Synthetic payment against the live testnet deployment (RCS-112).
 *
 * The one test in the suite that exercises the money path for real: it creates
 * an invoice over the API, broadcasts an actual Sepolia transaction to the
 * address the server derived, waits for `paid` on the public checkout
 * WebSocket, and asserts the store webhook fired with a valid signature.
 * Everything else in the repo stops short of an on-chain payment.
 *
 * Run:
 *   E2E_REMOTE=true E2E_SYNTHETIC_PAYMENT=true npx playwright test tests/synthetic-payment.spec.ts
 *
 * Off by default — it spends real (testnet) ETH and needs secrets, so the
 * in-pipeline `e2e` job must not pick it up. When it *is* switched on, missing
 * configuration is a hard failure rather than a skip: a silently-skipped money
 * path is exactly the gap this ticket exists to close.
 *
 * Funds are recoverable. The store's xpub is `m/44'/60'/0'` of
 * `E2E_TEST_MNEMONIC`, so every address the server derives (`0/{index}`) is
 * spendable from the same mnemonic; the spender lives at a separate account
 * index and is topped up from a faucet.
 */
import { appendFileSync } from 'node:fs';

import { test, expect } from '@playwright/test';
import { createPublicClient, createWalletClient, formatEther, http, parseEther } from 'viem';
import { HDKey, mnemonicToAccount } from 'viem/accounts';
import { sepolia } from 'viem/chains';
import { mnemonicToSeedSync } from '@scure/bip39';

import { api, wsUrl } from '../fixtures/api';
import { WebhookSink, verifySignature } from '../fixtures/webhook-sink';

const ENABLED = process.env.E2E_SYNTHETIC_PAYMENT === 'true';

const CHAIN_ID = 11155111;
/** Account-level path the server expects an xpub at (`evm/src/wallet.rs`). */
const MERCHANT_PATH = "m/44'/60'/0'";
/** Kept clear of account 0 so the spender never collides with a receive address. */
const SPENDER_ACCOUNT_INDEX = 9;
/** Human units; Sepolia is 3 confirmations at ~12s blocks. */
const INVOICE_AMOUNT_ETH = '0.0001';
/** Headroom over the invoice amount for gas; a Sepolia transfer is well under this. */
const GAS_MARGIN_ETH = '0.0005';
const PAID_TIMEOUT_MS = 5 * 60_000;
const WEBHOOK_TIMEOUT_MS = 2 * 60_000;
// Sepolia inclusion is the one step whose latency we do not control. It is
// budgeted separately so the paid-detection window is not eaten by it.
const RECEIPT_TIMEOUT_MS = 3 * 60_000;
/**
 * Warn once the spender holds less than this many runs' worth (RCS-202).
 *
 * The hard guard below only trips when the wallet is already short for the
 * *current* run — a cliff, not a warning, whose first notice is a red nightly.
 * At ~0.00012 per run this gives weeks of notice instead.
 */
const LOW_BALANCE_RUNS = 20;

function requireEnv(name: string, why: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required when E2E_SYNTHETIC_PAYMENT=true — ${why}`);
  }
  return value;
}

interface PaymentOption {
  chain_id: number;
  asset_symbol: string;
  token_address: string | null;
  payment_address: string;
  amount: string;
}

interface Checkout {
  status: string;
  amount_received: string;
  is_paid: boolean;
  payments: { tx_hash: string }[];
}

/**
 * Subscribe to the public checkout socket and resolve once the invoice reports
 * `paid`.
 *
 * Reconnects on drop, because a five-minute wait outlives plenty of sockets.
 * The socket only carries live events — it replays nothing on connect — so a
 * transition landing inside a reconnect gap would otherwise be missed forever;
 * each gap is therefore closed by re-reading `GET /checkout/{id}`.
 */
async function waitForPaid(
  invoiceId: string,
  timeoutMs: number,
): Promise<{ via: string; seen: string[] }> {
  const seen: string[] = [];
  const deadline = Date.now() + timeoutMs;
  const url = `${wsUrl('/checkout/ws')}?invoice_id=${encodeURIComponent(invoiceId)}`;

  for (;;) {
    const paid = await new Promise<boolean>((resolve) => {
      let timer: ReturnType<typeof setTimeout>;
      const socket = new WebSocket(url);
      const settle = (value: boolean) => {
        clearTimeout(timer);
        socket.close();
        resolve(value);
      };
      timer = setTimeout(() => settle(false), Math.max(1_000, deadline - Date.now()));

      // Errors and drops both just end this attempt; only the deadline below
      // turns a run of failed attempts into a test failure.
      socket.onerror = () => settle(false);
      socket.onclose = () => settle(false);
      socket.onmessage = (event: { data: unknown }) => {
        const update = JSON.parse(String(event.data)) as { type: string; status?: string };
        seen.push(update.status ? `${update.type}:${update.status}` : update.type);
        if (update.type === 'invoice_status' && update.status === 'paid') settle(true);
      };
    });
    if (paid) return { via: 'checkout WebSocket', seen };

    const checkout = await api<Checkout>(`/checkout/${invoiceId}`).catch(() => null);
    if (checkout?.is_paid) return { via: 'checkout API after a WebSocket drop', seen };

    if (Date.now() >= deadline) {
      throw new Error(
        `Invoice ${invoiceId} did not reach 'paid' within ${timeoutMs}ms. ` +
          `WebSocket saw: [${seen.join(', ') || 'nothing'}]. ` +
          `Checkout API reports: ${checkout ? `${checkout.status} (received ${checkout.amount_received})` : 'unreachable'}. ` +
          `If it is stuck at 'pending' the chain monitors are probably not connected — ` +
          `check evmmonitor:health in Redis (RCS-187).`,
      );
    }
    await new Promise((r) => setTimeout(r, 2_000));
  }
}

test.describe('Synthetic payment (live testnet)', () => {
  // A retry would broadcast a second transaction and leave the first invoice
  // half-paid, so this suite never retries even when the rest of CI does.
  test.describe.configure({ retries: 0 });
  test.skip(
    !ENABLED,
    'Set E2E_SYNTHETIC_PAYMENT=true to run the on-chain payment test (spends testnet ETH)',
  );

  test('invoice → on-chain tx → paid → webhook', async () => {
    test.setTimeout(RECEIPT_TIMEOUT_MS + PAID_TIMEOUT_MS + WEBHOOK_TIMEOUT_MS + 5 * 60_000);

    const mnemonic = requireEnv('E2E_TEST_MNEMONIC', 'BIP39 phrase for the merchant xpub + spender');
    const token = requireEnv('E2E_API_TOKEN', 'API key (ak_...) that may create stores and invoices');
    const rpcUrl = requireEnv('E2E_SEPOLIA_RPC_URL', 'Sepolia RPC endpoint to broadcast from');

    const merchantXpub = HDKey.fromMasterSeed(mnemonicToSeedSync(mnemonic)).derive(MERCHANT_PATH)
      .publicExtendedKey;
    const spender = mnemonicToAccount(mnemonic, { accountIndex: SPENDER_ACCOUNT_INDEX });

    const transport = http(rpcUrl);
    const publicClient = createPublicClient({ chain: sepolia, transport });
    const walletClient = createWalletClient({ account: spender, chain: sepolia, transport });

    // Fail on an empty wallet with the address to refill, not with a stack
    // trace from deep inside viem when the transaction is rejected.
    // Against the amount actually needed plus a gas margin — `> 0n` passes with
    // 1 wei, which is exactly the near-drained wallet this guard exists for, and
    // the run would then die inside viem with an insufficient-funds trace.
    const balance = await publicClient.getBalance({ address: spender.address });
    const needed = parseEther(INVOICE_AMOUNT_ETH) + parseEther(GAS_MARGIN_ETH);
    expect(
      balance >= needed,
      `Test wallet ${spender.address} holds ${formatEther(balance)} SepoliaETH, ` +
        `needs at least ${formatEther(needed)}. Refill it from a faucet.`,
    ).toBe(true);
    console.log(`spender ${spender.address} — ${formatEther(balance)} SepoliaETH`);

    // Advance warning, never a failure: the run is fine, the wallet just needs
    // topping up before it isn't. Surfaces in the Actions summary so it is seen
    // without anyone reading the log (RCS-202).
    const lowWater = needed * BigInt(LOW_BALANCE_RUNS);
    if (balance < lowWater) {
      const runsLeft = Number(balance / needed);
      const msg =
        `Synthetic payment wallet is low: ${spender.address} holds ` +
        `${formatEther(balance)} SepoliaETH, about ${runsLeft} run(s) left. ` +
        `Top it up from a Sepolia faucet, or reclaim parked funds with ` +
        `\`node scripts/sweep-test-wallet.mjs\`.`;
      console.log(`::warning title=Synthetic payment wallet low::${msg}`);
      if (process.env.GITHUB_STEP_SUMMARY) {
        appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### \u26a0\ufe0f Wallet low\n\n${msg}\n`);
      }
    }

    const sink = await WebhookSink.start();
    try {
      console.log(`webhook sink listening on :${sink.port}, public at ${sink.publicUrl}`);

      // Fresh store per run: the derivation index advances per payment method,
      // so reusing one would couple today's run to yesterday's state. Stores are
      // left behind on purpose — a failed run's invoice is the evidence.
      const stamp = new Date().toISOString().replace(/[:.]/g, '-');
      const store = await api<{ id: string }>('/stores', {
        method: 'POST',
        token,
        body: { name: `e2e-synthetic-${stamp}` },
      });

      await api(`/stores/${store.id}/payment-methods`, {
        method: 'POST',
        token,
        body: {
          chain_id: CHAIN_ID,
          token_address: null,
          asset_symbol: 'ETH',
          decimals: 18,
          xpub: merchantXpub,
        },
      });

      const webhook = await api<{ webhook_secret: string | null }>(
        `/stores/${store.id}/webhook`,
        { method: 'PUT', token, body: { webhook_url: sink.publicUrl, enabled: true } },
      );
      const secret = webhook.webhook_secret;
      expect(secret, 'webhook secret is only returned on upsert — it must be present here').toBeTruthy();

      // ETH-denominated invoice: currency matches the asset, so no exchange
      // rate is involved and the test does not depend on the rate provider.
      const invoice = await api<{ id: string; payment_options: PaymentOption[] }>('/invoices', {
        method: 'POST',
        token,
        body: {
          store_id: store.id,
          currency: 'ETH',
          amount: INVOICE_AMOUNT_ETH,
          expiration_seconds: 1_800,
          metadata: { source: 'rcs-112-synthetic-payment' },
        },
      });

      const option = invoice.payment_options.find(
        (o) => o.chain_id === CHAIN_ID && o.token_address === null,
      );
      expect(option, `no native Sepolia payment option on invoice ${invoice.id}`).toBeDefined();
      const target = option as PaymentOption;
      console.log(`invoice ${invoice.id} → ${target.amount} wei to ${target.payment_address}`);

      // Subscribe before broadcasting: the socket only forwards live events, so
      // a fast confirmation must not land while we are still connecting.
      // The budget covers inclusion as well as detection: this starts before
      // the broadcast (deliberately — the socket only forwards live events),
      // so a slow Sepolia block would otherwise spend most of PAID_TIMEOUT_MS
      // before the monitors have anything to detect, and the failure would be
      // reported as "chain monitors are probably not connected".
      const paidPromise = waitForPaid(invoice.id, RECEIPT_TIMEOUT_MS + PAID_TIMEOUT_MS);
      // Mark it handled now: if an assertion below throws first, an unobserved
      // rejection here would take the worker down instead of reporting.
      paidPromise.catch(() => {});

      const hash = await walletClient.sendTransaction({
        to: target.payment_address as `0x${string}`,
        value: BigInt(target.amount),
      });
      console.log(`sent https://sepolia.etherscan.io/tx/${hash}`);
      const receipt = await publicClient.waitForTransactionReceipt({ hash, timeout: RECEIPT_TIMEOUT_MS });
      expect(receipt.status, `transaction ${hash} reverted`).toBe('success');

      const { via, seen } = await paidPromise;
      console.log(`paid, detected via ${via} — updates: ${seen.join(', ') || 'none'}`);

      const checkout = await api<Checkout>(`/checkout/${invoice.id}`);
      expect(checkout.is_paid).toBe(true);
      expect(checkout.payments.map((p) => p.tx_hash.toLowerCase())).toContain(hash.toLowerCase());

      const delivered = await sink.waitFor('payment_confirmed', WEBHOOK_TIMEOUT_MS);
      expect(delivered.body.invoice_id).toBe(invoice.id);
      expect(delivered.body.store_id).toBe(store.id);
      expect(delivered.body.status).toBe('paid');
      expect(delivered.body.chain_id).toBe(CHAIN_ID);
      expect(
        verifySignature(delivered, secret as string),
        'X-Webhook-Signature did not match an HMAC-SHA256 of the delivered body',
      ).toBe(true);
    } finally {
      await sink.stop();
    }
  });
});
