#!/usr/bin/env python3
"""Populate the RPC endpoints in the *runner's* testnet .env (RCS-187).

The `deploy-testnet` CI job runs as `ghrunner`, so its `cd ~/deploy/...` resolves
to /home/ghrunner/deploy/ethpayserver/testnet — not /home/gus/deploy/..., which
is a stale copy. The runner's .env has empty EVMMONITOR_CHAIN_*_RPC_* values, so
every CI deploy brings the stack up with no RPC endpoints and all five chain
monitors die, while the deploy job still reports success.

Run with sudo (only root can read/write ghrunner's home):

    sudo python3 scripts/fix-runner-testnet-env.py

Rewrites ONLY the ten EVMMONITOR_CHAIN_*_RPC_{HTTP,WS} keys, taking their values
from the filled copy. Every other line — POSTGRES_PASSWORD above all, which the
database was initialised with — is preserved byte for byte. Ownership and mode
are preserved. Secrets are never printed.
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

SOURCE = Path("/home/gus/deploy/ethpayserver/testnet/.env")
TARGET = Path("/home/ghrunner/deploy/ethpayserver/testnet/.env")
PREFIX = "EVMMONITOR_CHAIN_"
SUFFIXES = ("_RPC_HTTP", "_RPC_WS")


def rpc_keys(lines: list[str]) -> dict[str, str]:
    found = {}
    for line in lines:
        if not line.startswith(PREFIX) or "=" not in line:
            continue
        key, _, value = line.partition("=")
        if key.endswith(SUFFIXES) and value.strip():
            found[key] = value.rstrip("\n")
    return found


def main() -> int:
    if os.geteuid() != 0:
        print("must run as root (sudo) — ghrunner's home is mode 700", file=sys.stderr)
        return 1
    for path in (SOURCE, TARGET):
        if not path.is_file():
            print(f"missing: {path}", file=sys.stderr)
            return 1

    values = rpc_keys(SOURCE.read_text().splitlines())
    if len(values) != 10:
        print(f"expected 10 filled RPC keys in {SOURCE}, found {len(values)}", file=sys.stderr)
        return 1

    original = TARGET.read_text().splitlines(keepends=True)
    stat = TARGET.stat()
    shutil.copy2(TARGET, TARGET.with_suffix(".env.bak"))

    out, replaced, seen = [], 0, set()
    for line in original:
        key = line.partition("=")[0]
        if key in values:
            out.append(values[key] + "\n")
            seen.add(key)
            replaced += 1
        else:
            out.append(line)

    missing = [k for k in values if k not in seen]
    if missing:
        if out and not out[-1].endswith("\n"):
            out.append("\n")
        out.extend(values[k] + "\n" for k in missing)

    TARGET.write_text("".join(out))
    os.chown(TARGET, stat.st_uid, stat.st_gid)
    os.chmod(TARGET, stat.st_mode & 0o7777)

    print(f"replaced {replaced}, appended {len(missing)} RPC keys in {TARGET}")
    print(f"backup: {TARGET.with_suffix('.env.bak')}")
    print("other keys untouched. now redeploy, or restart the monitor:")
    print("  cd /home/ghrunner/deploy/ethpayserver/testnet && docker compose up -d evmmonitor")
    return 0


if __name__ == "__main__":
    sys.exit(main())
