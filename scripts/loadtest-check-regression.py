#!/usr/bin/env python3
"""Compare load test results against baselines and flag regressions.

Reads the JSON output from the loadtest and loadtest-ws binaries, compares
each metric against the baselines in loadtest/baselines.json, and exits
non-zero if any metric has regressed beyond the configured threshold.

Usage:
    loadtest-check-regression.py --http results-http.json [--ws results-ws.json]

Exit codes:
    0 — all metrics within tolerance
    1 — one or more regressions detected
    2 — usage / file error
"""

import argparse
import json
import sys
from pathlib import Path

BASELINES_PATH = Path(__file__).resolve().parent.parent / "loadtest" / "baselines.json"


def load_json(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


def check_http(results: dict, baselines: dict, threshold_pct: float) -> list[str]:
    """Check HTTP scenario results against baselines. Returns list of failures."""
    failures = []
    scenarios = baselines.get("http_scenarios", {})
    requests = {r["name"]: r for r in results.get("requests", [])}

    for name, baseline in scenarios.items():
        actual = requests.get(name)
        if actual is None:
            print(f"  WARNING: baseline scenario '{name}' not found in results", file=sys.stderr)
            continue

        # p95 latency check (higher is worse)
        if "p95_ms" in baseline:
            target = baseline["p95_ms"]
            limit = target * (1 + threshold_pct / 100)
            actual_p95 = actual["p95_ms"]
            if actual_p95 > limit:
                pct_over = ((actual_p95 - target) / target) * 100
                failures.append(
                    f"  {name}: p95={actual_p95}ms exceeds "
                    f"baseline {target}ms by {pct_over:.1f}% "
                    f"(threshold: {threshold_pct}%)"
                )

        # RPS check (lower is worse)
        if "min_rps" in baseline:
            target = baseline["min_rps"]
            limit = target * (1 - threshold_pct / 100)
            actual_rps = actual["rps"]
            if actual_rps < limit:
                pct_under = ((target - actual_rps) / target) * 100
                failures.append(
                    f"  {name}: rps={actual_rps:.2f} is "
                    f"{pct_under:.1f}% below baseline {target:.2f} "
                    f"(threshold: {threshold_pct}%)"
                )

    return failures


def check_ws(results: dict, baselines: dict, threshold_pct: float) -> list[str]:
    """Check WebSocket results against baselines. Returns list of failures."""
    failures = []
    ws_baseline = baselines.get("websocket", {})

    if "max_disconnect_rate_pct" in ws_baseline:
        target = ws_baseline["max_disconnect_rate_pct"]
        limit = target * (1 + threshold_pct / 100)
        actual_rate = results.get("disconnect_rate_pct", 0)
        if actual_rate > limit:
            pct_over = ((actual_rate - target) / target) * 100 if target > 0 else float("inf")
            failures.append(
                f"  websocket: disconnect_rate={actual_rate:.2f}% exceeds "
                f"baseline {target}% by {pct_over:.1f}% "
                f"(threshold: {threshold_pct}%)"
            )

    return failures


def main():
    parser = argparse.ArgumentParser(description="Load test regression checker")
    parser.add_argument("--http", required=True, help="Path to HTTP load test JSON results")
    parser.add_argument("--ws", help="Path to WebSocket load test JSON results")
    parser.add_argument(
        "--baselines",
        default=str(BASELINES_PATH),
        help="Path to baselines JSON (default: loadtest/baselines.json)",
    )
    args = parser.parse_args()

    try:
        baselines = load_json(args.baselines)
    except (FileNotFoundError, json.JSONDecodeError) as e:
        print(f"Error loading baselines: {e}", file=sys.stderr)
        sys.exit(2)

    threshold = baselines.get("regression_threshold_pct", 25)
    failures = []

    try:
        http_results = load_json(args.http)
        failures.extend(check_http(http_results, baselines, threshold))
    except (FileNotFoundError, json.JSONDecodeError) as e:
        print(f"Error loading HTTP results: {e}", file=sys.stderr)
        sys.exit(2)

    if args.ws:
        try:
            ws_results = load_json(args.ws)
            failures.extend(check_ws(ws_results, baselines, threshold))
        except (FileNotFoundError, json.JSONDecodeError) as e:
            print(f"Error loading WS results: {e}", file=sys.stderr)
            sys.exit(2)

    if failures:
        print(f"REGRESSION DETECTED ({len(failures)} metric(s) exceeded {threshold}% threshold):\n")
        for f in failures:
            print(f)
        print(f"\nBaselines: {args.baselines}")
        sys.exit(1)
    else:
        print(f"All metrics within {threshold}% of baseline targets.")
        sys.exit(0)


if __name__ == "__main__":
    main()
