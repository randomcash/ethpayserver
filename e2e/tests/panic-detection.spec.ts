import { test, expect } from '@playwright/test';
import { isClientPanic } from '../fixtures/auth';

// Pure unit tests, same style as the compareTiming tests in perf.spec.ts.
// The point of this predicate is that it fires on Rust panics and on nothing
// else: too narrow and RCS-220 recurs unnoticed, too broad and the first noisy
// 404 gets the whole check disabled.
test.describe('client panic detection', () => {
  test('catches a release-build trap, which carries no message', () => {
    expect(isClientPanic('unreachable')).toBe(true);
    expect(isClientPanic('RuntimeError: unreachable')).toBe(true);
  });

  test('catches a debug-build panic with its location', () => {
    expect(
      isClientPanic(
        'panicked at reactive_graph-0.1.8/src/traits.rs:388:39:\nTried to access a reactive value that has already been disposed.',
      ),
    ).toBe(true);
  });

  test('ignores the console noise this app actually produces', () => {
    for (const benign of [
      'Failed to load resource: the server responded with a status of 404 (Not Found)',
      "WebSocket connection to 'ws://localhost:8080/api/checkout/ws?invoice_id=0' failed: Error during WebSocket handshake: Unexpected response code: 400",
      'Failed to load resource: the server responded with a status of 429 ()',
      '[AuthContext] validate_session: Error - HTTP error 500: Internal Server Error',
    ]) {
      expect(isClientPanic(benign), benign).toBe(false);
    }
  });
});
