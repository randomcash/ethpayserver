import * as fs from 'fs';
import * as path from 'path';
import type {
  FullResult,
  Reporter,
  TestCase,
  TestResult,
} from '@playwright/test/reporter';
import { compareTiming, loadBaselines } from './fixtures/perf';

const BASELINES_PATH = path.join(__dirname, 'perf-baselines.json');
const RESULTS_PATH = path.join(__dirname, 'perf-results.json');

interface TestTiming {
  fullTitle: string;
  duration: number;
}

class PerfReporter implements Reporter {
  private timings: TestTiming[] = [];
  private tolerancePct: number;

  constructor() {
    this.tolerancePct = parseInt(process.env.PERF_TOLERANCE_PCT || '20', 10);
  }

  onTestEnd(test: TestCase, result: TestResult): void {
    this.timings.push({
      fullTitle: test.titlePath().join(' > '),
      duration: result.duration,
    });
  }

  async onEnd(
    _result: FullResult,
  ): Promise<{ status?: FullResult['status'] } | void> {
    // Write current run timings
    const resultMap: Record<string, number> = {};
    for (const t of this.timings) {
      resultMap[t.fullTitle] = t.duration;
    }
    fs.writeFileSync(RESULTS_PATH, JSON.stringify(resultMap, null, 2) + '\n');

    // Load baselines and compare
    const baselines = loadBaselines(BASELINES_PATH);
    const regressions: string[] = [];

    for (const timing of this.timings) {
      const baseline = baselines[timing.fullTitle];
      const { pass, delta } = compareTiming(
        timing.duration,
        baseline,
        this.tolerancePct,
      );

      if (!pass && delta !== null) {
        regressions.push(
          `  "${timing.fullTitle}" took ${timing.duration}ms ` +
            `(+${delta.toFixed(1)}% vs maxAllowed ${baseline!.maxAllowed}ms)`,
        );
      }
    }

    if (regressions.length > 0) {
      console.error('\nPerformance regressions detected:\n');
      for (const r of regressions) {
        console.error(r);
      }
      console.error(
        `\n  Tolerance: ${this.tolerancePct}% (PERF_TOLERANCE_PCT)\n`,
      );
      return { status: 'failed' };
    }

    console.log('\n  Performance: all tests within baseline thresholds.\n');
  }
}

export default PerfReporter;
