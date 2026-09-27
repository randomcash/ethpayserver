//! Exercises the `derive-xpub` binary itself, not just the library functions
//! it calls.
//!
//! Every unit test for the underlying derivation lives in `evm::wallet` and
//! calls `HdWallet` directly - none of them run through the binary's argv
//! dispatch, its two sequential stdin reads (mnemonic, then passphrase) for
//! `from-existing`, or its printed output. That CLI glue is the literal
//! entry point the onboarding docs tell every merchant to run, so a bug in
//! argument matching, stdin read order, or which field gets printed would
//! reach a merchant with zero test coverage catching it first.

use evm::{ChainFamily, HdWallet};
use std::io::Write;
use std::process::{Command, Stdio};

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// Matches `EVM_ACCOUNT_XPUB` in `evm/src/wallet.rs`'s test module - kept as
/// an independent literal rather than importing the library constant, so
/// this test still catches a regression that broke the constant itself.
const EVM_ACCOUNT_XPUB: &str = "xpub6DCoCpSuQZB2jawqnGMEPS63ePKWkwWPH4TU45Q7LPXWuNd8TMtVxRrgjtEshuqpK3mdhaWHPFsBngh5GFZaM6si3yZdUsT8ddYM3PwnATt";

/// Matches `EVM_ACCOUNT_XPUB_WITH_PASSPHRASE` in `evm/src/wallet.rs`.
const EVM_ACCOUNT_XPUB_WITH_PASSPHRASE: &str = "xpub6Bmqz11Kt5qtj3xbXZkzEyYw43EDFGCon5GzC4udf7DPugyKjVppdX2amQZrGs4rqAJH79pDtge2UDENZzjz9DgcV3WmfbwYAXj2epC5cgz";

fn run_from_existing(mnemonic: &str, passphrase: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_derive-xpub"))
        .arg("from-existing")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn derive-xpub");

    let mut stdin = child.stdin.take().expect("stdin piped");
    write!(stdin, "{mnemonic}\n{passphrase}\n").expect("write to stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for derive-xpub");
    assert!(
        output.status.success(),
        "derive-xpub exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

/// The path a merchant pasting an existing seed phrase with no passphrase
/// actually walks: two lines on stdin (mnemonic, then a blank line), and the
/// xpub a real wallet would show must appear in the printed output.
#[test]
fn from_existing_with_no_passphrase_prints_the_expected_xpub() {
    let stdout = run_from_existing(TEST_MNEMONIC, "");
    assert!(
        stdout.contains(EVM_ACCOUNT_XPUB),
        "expected {EVM_ACCOUNT_XPUB} in output, got:\n{stdout}"
    );
}

/// The second stdin read is the passphrase line, not discarded or merged
/// into the first - if it were, this would print `EVM_ACCOUNT_XPUB` (the
/// empty-passphrase key) instead of the passphrase-protected one.
#[test]
fn from_existing_with_a_passphrase_prints_the_passphrase_protected_xpub() {
    let stdout = run_from_existing(TEST_MNEMONIC, "secret");
    assert!(
        stdout.contains(EVM_ACCOUNT_XPUB_WITH_PASSPHRASE),
        "expected {EVM_ACCOUNT_XPUB_WITH_PASSPHRASE} in output, got:\n{stdout}"
    );
    assert!(
        !stdout.contains(EVM_ACCOUNT_XPUB),
        "output contains the empty-passphrase xpub - the passphrase line was \
         dropped or ignored:\n{stdout}"
    );
}

/// A BIP-39 passphrase is used byte-for-byte, so leading/trailing spaces a
/// merchant genuinely typed must survive the two stdin reads intact - only
/// the trailing line ending `read_line` leaves behind may be stripped. If
/// this ever regressed to a general `.trim()`, this would silently start
/// deriving a different, wrong xpub with no error, and every other test in
/// this file (which use whitespace-free passphrases) would keep passing.
#[test]
fn from_existing_preserves_leading_and_trailing_passphrase_whitespace() {
    let padded_passphrase = "  secret  ";
    let expected_xpub = HdWallet::from_mnemonic(TEST_MNEMONIC, padded_passphrase)
        .expect("valid mnemonic")
        .account_xpub_string_for(ChainFamily::Evm)
        .expect("derive xpub");

    let stdout = run_from_existing(TEST_MNEMONIC, padded_passphrase);
    assert!(
        stdout.contains(&expected_xpub),
        "expected the whitespace-preserving xpub {expected_xpub} in output, got:\n{stdout}"
    );
    assert!(
        !stdout.contains(EVM_ACCOUNT_XPUB_WITH_PASSPHRASE),
        "output matches the trimmed-passphrase xpub - leading/trailing whitespace was \
         stripped instead of preserved:\n{stdout}"
    );
}

/// `generate` takes no stdin and must still succeed and print an
/// `xpub`-prefixed key - the argv dispatch branch `from-existing` above
/// doesn't exercise.
#[test]
fn generate_prints_an_xpub_prefixed_key() {
    let output = Command::new(env!("CARGO_BIN_EXE_derive-xpub"))
        .arg("generate")
        .output()
        .expect("run derive-xpub generate");
    assert!(
        output.status.success(),
        "derive-xpub generate exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("xpub:"),
        "expected an xpub line in output, got:\n{stdout}"
    );
    let xpub_line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with("xpub:"))
        .expect("xpub line present");
    let key = xpub_line
        .trim_start()
        .strip_prefix("xpub:")
        .expect("prefix checked above")
        .trim();
    assert!(
        key.starts_with("xpub"),
        "printed key doesn't look like an xpub-prefixed key: {xpub_line}"
    );
}

/// A merchant hitting Enter on an empty line (or piping an empty file) must
/// get a clean, loud failure - not a panic and not a plausible-looking xpub
/// derived from an empty string.
#[test]
fn from_existing_with_a_blank_mnemonic_fails_cleanly() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_derive-xpub"))
        .arg("from-existing")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn derive-xpub");

    let mut stdin = child.stdin.take().expect("stdin piped");
    writeln!(stdin).expect("write to stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for derive-xpub");
    assert!(
        !output.status.success(),
        "derive-xpub should reject a blank mnemonic, got:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(
        stderr.contains("no mnemonic given"),
        "expected the blank-mnemonic error, got:\n{stderr}"
    );
}

/// A typo'd or garbled seed phrase must fail loud, not silently derive a
/// plausible-looking but wrong xpub the merchant would register in good
/// faith - the same class of failure this tool exists to prevent.
#[test]
fn from_existing_with_an_invalid_mnemonic_fails_cleanly() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_derive-xpub"))
        .arg("from-existing")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn derive-xpub");

    let mut stdin = child.stdin.take().expect("stdin piped");
    write!(stdin, "not a valid bip39 mnemonic at all\n\n").expect("write to stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for derive-xpub");
    assert!(
        !output.status.success(),
        "derive-xpub should reject an invalid mnemonic, got:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(
        stderr.contains("invalid mnemonic"),
        "expected the invalid-mnemonic error, got:\n{stderr}"
    );
}
