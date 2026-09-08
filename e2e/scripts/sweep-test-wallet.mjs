#!/usr/bin/env node
/**
 * Reclaim funds parked by the synthetic-payment test (RCS-202).
 *
 * Each nightly run sends three payments of a random 0.00005-0.00015 ETH from
 * the spender (m/44'/60'/9'/0/0) to addresses the server derived from the
 * merchant xpub at m/44'/60'/0'/0/{i}. Both come from the same seed, so that
 * principal is never actually spent — only gas is. This walks the derived
 * addresses and sends anything worth moving back to the spender.
 *
 * The indices march outwards, three per night, and never restart. They used to:
 * the counter lived on the payment method, so a fresh store began at 0 and the
 * parked funds piled up on the first few addresses. RCS-234 moved the counter
 * onto an account-level wallet keyed by the xpub, which is the whole point -
 * one key, one counter, no reuse - so a fresh store now continues wherever that
 * key left off. The scan has to cover the entire history rather than a window
 * near zero, and a range that stops short does not fail: it silently reports
 * nothing to sweep while the funds sit past the end of it.
 *
 * Hence --scan 1000 by default, which is over three years of nightlies at three
 * payments a night. Raise it rather than trim it; each index is one RPC call.
 *
 * Deliberately NOT part of the test: a failed sweep must not fail a run whose
 * payment already succeeded, and best-effort cleanup inside an assertion is how
 * things quietly stop working. Run it occasionally instead — once a year is
 * plenty, since gas is the only real burn.
 *
 *   E2E_TEST_MNEMONIC="..." E2E_SEPOLIA_RPC_URL="https://..." \
 *     node scripts/sweep-test-wallet.mjs [--scan 1000] [--execute]
 *
 * Dry run by default: it prints what it would move and sends nothing. Pass
 * --execute to actually broadcast.
 */
import {
  createPublicClient,
  createWalletClient,
  formatEther,
  http,
} from 'viem';
import { mnemonicToAccount } from 'viem/accounts';
import { sepolia } from 'viem/chains';

const SPENDER_ACCOUNT_INDEX = 9; // must match synthetic-payment.spec.ts
const TRANSFER_GAS = 21_000n;
/** Skip anything that would cost more to move than it is worth. */
const GAS_HEADROOM = 2n; // require value >= 2x the fee before bothering

function requireEnv(name) {
  const v = process.env[name];
  if (!v) {
    console.error(`${name} is required. See e2e/README.md.`);
    process.exit(1);
  }
  return v;
}

const args = process.argv.slice(2);
const execute = args.includes('--execute');
const scanIdx = args.indexOf('--scan');
const scanCount = scanIdx >= 0 ? Number(args[scanIdx + 1]) : 1000;

const mnemonic = requireEnv('E2E_TEST_MNEMONIC');
const rpcUrl = requireEnv('E2E_SEPOLIA_RPC_URL');

const transport = http(rpcUrl);
const publicClient = createPublicClient({ chain: sepolia, transport });

const spender = mnemonicToAccount(mnemonic, { accountIndex: SPENDER_ACCOUNT_INDEX });
const gasPrice = await publicClient.getGasPrice();
const fee = TRANSFER_GAS * gasPrice;

console.log(`sweeping to spender ${spender.address}`);
console.log(`gas ${Number(gasPrice) / 1e9} gwei — a transfer costs ${formatEther(fee)} ETH`);
console.log(execute ? 'MODE: execute\n' : 'MODE: dry run (pass --execute to broadcast)\n');

let swept = 0n;
let moved = 0;

for (let i = 0; i < scanCount; i++) {
  // Matches the server's receive-address derivation: m/44'/60'/0'/0/{i}
  const account = mnemonicToAccount(mnemonic, {
    accountIndex: 0,
    changeIndex: 0,
    addressIndex: i,
  });
  const balance = await publicClient.getBalance({ address: account.address });
  if (balance === 0n) continue;

  if (balance < fee * GAS_HEADROOM) {
    console.log(`  skip  ${i.toString().padStart(3)} ${account.address} ${formatEther(balance)} (dust)`);
    continue;
  }

  const value = balance - fee;
  console.log(`  move  ${i.toString().padStart(3)} ${account.address} ${formatEther(value)} ETH`);
  swept += value;
  moved++;

  if (execute) {
    const wallet = createWalletClient({ account, chain: sepolia, transport });
    const hash = await wallet.sendTransaction({
      to: spender.address,
      value,
      gas: TRANSFER_GAS,
      gasPrice,
    });
    await publicClient.waitForTransactionReceipt({ hash, timeout: 180_000 });
    console.log(`        sent https://sepolia.etherscan.io/tx/${hash}`);
  }
}

console.log(
  `\n${moved} address(es), ${formatEther(swept)} SepoliaETH ` +
    `${execute ? 'returned to' : 'would return to'} ${spender.address}`,
);
if (!execute && moved > 0) console.log('Re-run with --execute to broadcast.');
