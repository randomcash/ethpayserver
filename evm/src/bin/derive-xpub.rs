//! Offline tool to generate or derive the extended public key (xpub) a
//! merchant registers with `POST /wallets`.
//!
//! Links no HTTP client, so there is no code path here that could transmit a
//! mnemonic or a key anywhere. Run it disconnected if that isn't reassurance
//! enough on its own.
//!
//! ```text
//! derive-xpub generate       # new dedicated receiving wallet
//! derive-xpub from-existing  # derive from a mnemonic already held
//! ```

use evm::{ChainFamily, HdWallet, generate_mnemonic};
use std::io::BufRead;
use std::process::ExitCode;

/// Every payment this server derives is Ethereum/EVM; the tool has no reason
/// to ask which family, so it never does.
const FAMILY: ChainFamily = ChainFamily::Evm;

/// Addresses printed for comparison against a wallet's own display and
/// against `verification_addresses` in the `POST /wallets` response.
const CHECK_ADDRESS_COUNT: u32 = 3;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("generate") => generate(),
        Some("from-existing") => from_existing(),
        _ => {
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: derive-xpub <generate|from-existing>\n\n\
         generate       creates a brand new mnemonic and prints its account xpub.\n\
         from-existing  reads a mnemonic from stdin and prints its account xpub.\n\n\
         Makes no network connections either way."
    );
}

fn generate() -> ExitCode {
    let mnemonic = match generate_mnemonic(24) {
        Ok(m) => m,
        Err(e) => return fail(&format!("could not generate a mnemonic: {e}")),
    };
    println!(
        "New mnemonic - write it down somewhere offline and never type it into a \
         website:\n\n  {mnemonic}\n"
    );
    println!(
        "This wallet exists to receive payments and nothing else. Don't reuse it \
         as a daily-driver wallet, and don't import it into a browser extension."
    );
    emit(&mnemonic)
}

fn from_existing() -> ExitCode {
    eprintln!(
        "Paste the mnemonic on one line and press Enter. Do this only on a \
         machine you trust, offline if you can manage it - typing a seed phrase \
         into software is exactly the shape of a wallet-drain attack, and this \
         tool is not an exception to that."
    );
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return fail("could not read from stdin");
    }
    let mnemonic = line.trim();
    if mnemonic.is_empty() {
        return fail("no mnemonic given");
    }
    emit(mnemonic)
}

fn emit(mnemonic: &str) -> ExitCode {
    let wallet = match HdWallet::from_mnemonic(mnemonic, "") {
        Ok(w) => w,
        Err(e) => return fail(&format!("invalid mnemonic: {e}")),
    };
    let xpub = match wallet.account_xpub_string_for(FAMILY) {
        Ok(x) => x,
        Err(e) => return fail(&format!("could not derive xpub: {e}")),
    };
    println!("\naccount path: {}", FAMILY.account_path());
    println!("xpub:         {xpub}\n");
    println!(
        "First {CHECK_ADDRESS_COUNT} receiving addresses - compare these against your \
         own wallet, and against `verification_addresses` from `POST /wallets`, \
         before a single invoice quotes one:\n"
    );
    print_check_addresses(&wallet)
}

fn print_check_addresses(wallet: &HdWallet) -> ExitCode {
    for index in 0..CHECK_ADDRESS_COUNT {
        match wallet.derive_address_for(FAMILY, index) {
            Ok(address) => println!(
                "  [{index}] {} ({})",
                FAMILY.encode_address(address),
                FAMILY.derivation_path(index)
            ),
            Err(e) => return fail(&format!("could not derive address {index}: {e}")),
        }
    }
    ExitCode::SUCCESS
}

fn fail(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::FAILURE
}
