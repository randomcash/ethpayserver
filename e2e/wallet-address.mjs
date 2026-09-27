// Where the synthetic-payment spec spends from, and what it registers.
//
// Both are derived from E2E_TEST_MNEMONIC, so they change when that secret
// changes and are written down nowhere. The spender sits at account index 9,
// deliberately clear of account 0 so it never collides with a receive address.
import { HDKey, mnemonicToAccount } from 'viem/accounts';
import { mnemonicToSeedSync } from '@scure/bip39';

const m = process.env.E2E_TEST_MNEMONIC;
if (!m) {
  console.error('E2E_TEST_MNEMONIC is not set');
  process.exit(1);
}

const xpub = HDKey.fromMasterSeed(mnemonicToSeedSync(m)).derive("m/44'/60'/0'").publicExtendedKey;
const spender = mnemonicToAccount(m, { accountIndex: 9 }).address;

console.log('');
console.log('receive xpub (registered by the spec):', xpub);
console.log('FUND THIS ADDRESS with SepoliaETH   :', spender);
console.log('');
console.log('about 0.002 SepoliaETH per nightly run; send a few weeks worth.');
