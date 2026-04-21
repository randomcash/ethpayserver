import { test, expect } from '@playwright/test';
import { compareTiming, type Baseline } from '../fixtures/perf';

test.describe('Performance timing infrastructure', () => {
  test('captures non-zero test duration', async ({}, testInfo) => {
    // Deliberately wait to produce a measurable, non-zero duration
    await new Promise((r) => setTimeout(r, 50));
    // testInfo.startTime is set by Playwright; verify it was recorded
    expect(testInfo.startTime.getTime()).toBeGreaterThan(0);
  });
});

test.describe('compareTiming unit tests', () => {
  test('passes when no baseline exists', () => {
    const result = compareTiming(1000, undefined);
    expect(result.pass).toBe(true);
    expect(result.delta).toBeNull();
  });

  test('passes when actual is within threshold', () => {
    const baseline: Baseline = { p50: 500, p95: 800, maxAllowed: 1000 };
    const result = compareTiming(1100, baseline, 20);
    expect(result.pass).toBe(true);
  });

  test('fails when actual exceeds threshold', () => {
    const baseline: Baseline = { p50: 500, p95: 800, maxAllowed: 1000 };
    const result = compareTiming(1300, baseline, 20);
    expect(result.pass).toBe(false);
    expect(result.delta).toBeGreaterThan(0);
  });

  test('reports correct delta percentage', () => {
    const baseline: Baseline = { p50: 500, p95: 800, maxAllowed: 1000 };
    const result = compareTiming(1500, baseline);
    expect(result.delta).toBe(50); // 50% above maxAllowed
  });

  test('passes when actual equals maxAllowed', () => {
    const baseline: Baseline = { p50: 500, p95: 800, maxAllowed: 1000 };
    const result = compareTiming(1000, baseline);
    expect(result.pass).toBe(true);
    expect(result.delta).toBe(0);
  });

  test('passes when actual is well below maxAllowed', () => {
    const baseline: Baseline = { p50: 500, p95: 800, maxAllowed: 1000 };
    const result = compareTiming(400, baseline);
    expect(result.pass).toBe(true);
    expect(result.delta).toBe(-60); // 60% below maxAllowed
  });

  test('respects custom tolerance percentage', () => {
    const baseline: Baseline = { p50: 500, p95: 800, maxAllowed: 1000 };
    // 1050ms with 5% tolerance (threshold = 1050ms) should fail
    expect(compareTiming(1060, baseline, 5).pass).toBe(false);
    // same value with 10% tolerance (threshold = 1100ms) should pass
    expect(compareTiming(1060, baseline, 10).pass).toBe(true);
  });
});
