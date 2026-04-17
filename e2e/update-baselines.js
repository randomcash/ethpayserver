#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

const resultsPath = path.join(__dirname, 'perf-results.json');
const baselinesPath = path.join(__dirname, 'perf-baselines.json');

if (!fs.existsSync(resultsPath)) {
  console.error(
    'No perf-results.json found. Run tests first:\n  npx playwright test\n',
  );
  process.exit(1);
}

const results = JSON.parse(fs.readFileSync(resultsPath, 'utf-8'));
const tolerance = parseInt(process.env.PERF_TOLERANCE_PCT || '20', 10);
const baselines = {};

for (const [title, duration] of Object.entries(results)) {
  baselines[title] = {
    p50: duration,
    p95: duration,
    maxAllowed: Math.ceil(duration * (1 + tolerance / 100)),
  };
}

fs.writeFileSync(baselinesPath, JSON.stringify(baselines, null, 2) + '\n');
console.log(
  `Updated baselines for ${Object.keys(baselines).length} tests (tolerance: ${tolerance}%)`,
);
