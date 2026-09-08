/**
 * Pure helpers for `tests/synthetic-payment.spec.ts`.
 *
 * They live outside the spec so they can be exercised without a mnemonic, an
 * RPC endpoint or a funded wallet. The spec itself only runs with
 * `E2E_SYNTHETIC_PAYMENT=true`, so arithmetic left inside it is unverified
 * until a nightly spends real ETH to find out — see
 * `tests/synthetic-payment-helpers.spec.ts`, which runs everywhere.
 */
import { parseEther } from 'viem';

/**
 * Bounds of the randomised invoice amount, in wei (inclusive).
 *
 * Centred on 0.0001 ETH — the amount this test paid when it was a constant —
 * so the expected spend per payment is unchanged and the wallet's burn rate is
 * comparable across the change. The band is ±50%: wide enough that anything
 * keyed to a hard-coded 0.0001 stops matching, still far above dust and far
 * below the point where a run's cost is worth thinking about.
 */
export const MIN_INVOICE_AMOUNT_WEI = parseEther('0.00005');
export const MAX_INVOICE_AMOUNT_WEI = parseEther('0.00015');

/**
 * Floor on the per-payment gas headroom.
 *
 * A floor, not the figure: the guard below takes the larger of this and what
 * the chain is actually charging right now. A constant alone is only a bound
 * while gas stays under it, and a sustained Sepolia spike above ~24 gwei made
 * the "worst case" an underestimate — the exact mid-run drain the guard says
 * it prevents. Kept as a floor because a momentarily cheap block should not
 * shrink the reserve either.
 */
const GAS_MARGIN_ETH = '0.0005';
/** A plain value transfer. Nothing here calls a contract. */
const TRANSFER_GAS = 21_000n;
/**
 * Multiplier on the live gas price when sizing the reserve.
 *
 * The price is read once, before the first broadcast, and the last payment is
 * sent up to eighteen minutes later. Reserving exactly today's price leaves no
 * room for the rise that happens in between, which is the case that drains the
 * wallet halfway through.
 */
const GAS_SPIKE_HEADROOM = 3n;
/**
 * Granularity of the draw: 1 gwei.
 *
 * Precision is not the reason. Every value on any grid round-trips exactly —
 * the chain takes wei, `invoices.amount` is NUMERIC(78, 18) and
 * `payment_options.amount` NUMERIC(78, 0) — and `formatEther` never renders in
 * scientific notation. The grid is for the humans reading a failed run: nine
 * decimal places stay legible in a log line and on Etherscan, and survive a
 * renderer that shows fewer than eighteen. It still leaves 100_001 distinct
 * amounts, so two invoices in one run drawing the same figure is a curiosity
 * (~3e-5) and not a problem: attribution here is by address and tx hash, never
 * by amount.
 */
export const AMOUNT_STEP_WEI = 1_000_000_000n;

/** Number of grid points in the band; the draw picks one of `STEPS + 1`. */
const STEPS = (MAX_INVOICE_AMOUNT_WEI - MIN_INVOICE_AMOUNT_WEI) / AMOUNT_STEP_WEI;

/**
 * Draw an invoice amount, uniform over the grid above.
 *
 * `random` is injectable so the unit test can pin both ends of the band rather
 * than sampling and hoping; a source that returns exactly 1 (`Math.random`
 * never does, a stub or a future replacement might) is clamped instead of
 * walking one step past the maximum the balance guard was sized for.
 */
export function randomInvoiceAmountWei(random: () => number = Math.random): bigint {
  const steps = Number(STEPS);
  const draw = Math.floor(random() * (steps + 1));
  const step = Math.min(steps, Math.max(0, draw));
  return MIN_INVOICE_AMOUNT_WEI + BigInt(step) * AMOUNT_STEP_WEI;
}

/**
 * The most one run can cost the wallet, at a given gas price.
 *
 * The maximum draw, not the average: the amount is random per invoice now, and
 * a guard sized on the average passes a wallet that a run of high draws then
 * drains mid-flight — three invoices in, two paid, one address left holding a
 * partial payment. `MAX_INVOICE_AMOUNT_WEI` is the ceiling the generator is
 * clamped to, so the amount half of this is an upper bound and not a hope.
 *
 * The gas half has to be measured, or it is only an upper bound while the
 * network agrees with a constant written months ago.
 */
export function worstCaseRunCostWei(paymentCount: number, gasPriceWei: bigint): bigint {
  const live = gasPriceWei * TRANSFER_GAS * GAS_SPIKE_HEADROOM;
  const floor = parseEther(GAS_MARGIN_ETH);
  const perPaymentGas = live > floor ? live : floor;
  return BigInt(paymentCount) * (MAX_INVOICE_AMOUNT_WEI + perPaymentGas);
}

/**
 * Clamp a per-step wait to what is left of a shared wall clock.
 *
 * Three payments in a run each carry the full per-payment timeouts, but they
 * do not each get to *spend* them: 3 × 10 minutes is the entire 30-minute job
 * cap in `.github/workflows/e2e-scheduled.yml`, and a job killed at that cap
 * uploads no report and names no step. Clamping every wait to the remaining
 * budget means a run that is going to run out fails inside Playwright, naming
 * the invoice it was waiting on.
 *
 * Never returns zero: a wait of 0ms reports "timed out" before it has looked
 * once, which reads as a server that answered wrongly rather than a run that
 * ran out of clock.
 */
export function remainingBudgetMs(deadlineAt: number, want: number, now = Date.now()): number {
  return Math.max(1_000, Math.min(want, deadlineAt - now));
}
