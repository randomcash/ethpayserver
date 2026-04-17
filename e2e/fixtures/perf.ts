import * as fs from 'fs';
import * as path from 'path';

export interface Baseline {
  p50: number;
  p95: number;
  maxAllowed: number;
}

export type BaselineMap = Record<string, Baseline>;

export interface CompareResult {
  pass: boolean;
  delta: number | null;
}

/**
 * Compare an actual timing against a baseline.
 * Fails if actual exceeds maxAllowed * (1 + tolerancePct / 100).
 * Returns delta as percentage above/below maxAllowed.
 */
export function compareTiming(
  actual: number,
  baseline: Baseline | undefined,
  tolerancePct: number = 20,
): CompareResult {
  if (!baseline) {
    return { pass: true, delta: null };
  }
  const threshold = baseline.maxAllowed * (1 + tolerancePct / 100);
  const delta =
    baseline.maxAllowed > 0
      ? Math.round(((actual - baseline.maxAllowed) / baseline.maxAllowed) * 1000) / 10
      : null;
  return {
    pass: actual <= threshold,
    delta,
  };
}

export function loadBaselines(baselinesPath?: string): BaselineMap {
  const p = baselinesPath || path.join(__dirname, '..', 'perf-baselines.json');
  if (!fs.existsSync(p)) return {};
  return JSON.parse(fs.readFileSync(p, 'utf-8'));
}
