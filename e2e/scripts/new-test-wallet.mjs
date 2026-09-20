#!/usr/bin/env node
/**
 * Mint a throwaway Sepolia wallet for the synthetic-payment suite.
 *
 * `e2e/README.md` has told you to run this since 2026-08-23. It did not
 * exist: the original was written, run twice, and never committed, so the
 * documented recovery path was a dead link for the one situation it was
 * written for. The phrase it printed was recovered from a tmux scrollback
 * once, and then lost with the session.
 *
 * Hence the emphasis below on writing the phrase down *before* touching
 * anything else. A wallet whose phrase exists only in a terminal is a wallet
 * with a deadline.
 *
 * One seed covers both sides of the test, which is why the suite needs only
 * one secret:
 *
 *   spender  m/44'/60'/9'/0/0   funds the payments
 *   merchant m/44'/60'/0'       the store xpub; receive addresses hang off
 *                               0/{i} below it
 *
 * Both come from the same seed on purpose - the principal is never really
 * spent, only gas is, so the wallet is a closed loop that needs topping up
 * for fees rather than for volume.
 *
 *   node scripts/new-test-wallet.mjs
 *
 * Prints and stores nothing. Piping it to a file is how the phrase ends up
 * somewhere it should not be; put it in a password manager by hand.
 */
import { english, generateMnemonic, mnemonicToAccount } from 'viem/accounts';

const SPENDER_ACCOUNT_INDEX = 9; // must match synthetic-payment.spec.ts

// 24 words. The old wallet was 12, which is 128 bits and fine; this costs
// nothing to strengthen and the phrase is typed once, into a secret field.
const mnemonic = generateMnemonic(english, 256);

const spender = mnemonicToAccount(mnemonic, { accountIndex: SPENDER_ACCOUNT_INDEX });
const merchant = mnemonicToAccount(mnemonic, { accountIndex: 0 });
const xpub = merchant.getHdKey().publicExtendedKey;

console.log(`
  Write the phrase into a password manager NOW, before anything else.
  It is not saved here, it is not in your shell history, and the last one
  was lost exactly this way.

  E2E_TEST_MNEMONIC
  ${mnemonic}

  Fund this address with Sepolia ETH — it pays for every synthetic payment:

  spender          ${spender.address}

  Set this as the store's payment-method xpub. It changes when the mnemonic
  changes, so rotating the secret without updating it sends the nightly's
  payments to addresses nobody is watching:

  merchant xpub    ${xpub}

  Then:
    gh secret set E2E_TEST_MNEMONIC   # paste at the prompt, do not pass -b
`);
