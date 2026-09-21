#!/usr/bin/env node
/**
 * Look at every screenshot tests/visual-review.spec.ts just took and report
 * what is wrong with it — the layout bugs a deterministic assertion cannot
 * see: content overflowing its container, an unlabelled control, a raw
 * decimal where a formatted amount belongs. A human found every one of these
 * on a phone; this is the agent that replaces that phone, not a new CI gate.
 *
 *   ANTHROPIC_API_KEY=... node scripts/visual-review.mjs
 *
 * Reads test-results/visual/manifest.json (written by the spec), writes
 * test-results/visual/findings.json ({ findings, errors }) and
 * test-results/visual/report.md. Advisory only: this always exits 0. A model
 * judging layout will produce false positives, and a check that can fail on
 * a false positive gets disabled — see scripts/health-gate.sh's history for
 * what that looks like.
 *
 * "errors" is not "findings": a route that could not be captured or
 * reviewed goes there, so it never renders the same as a route that was
 * looked at and found clean.
 */
import * as fs from 'node:fs';
import * as path from 'node:path';

const VISUAL_DIR = path.join('test-results', 'visual');
const MANIFEST_PATH = path.join(VISUAL_DIR, 'manifest.json');
const MODEL = process.env.VISUAL_REVIEW_MODEL || 'claude-sonnet-5';
const API_URL = 'https://api.anthropic.com/v1/messages';

const RUBRIC = `You are doing a nightly visual QA pass on a non-custodial crypto payment
processor's merchant dashboard. You are shown full-page screenshots of one
route: first at a mobile viewport (375x812), then at a desktop viewport
(1280x720) — a screenshot may be missing if that capture failed, in which
case just review the one you have.

Look only for concrete, visible defects — the kind a deterministic test
cannot catch:
- content clipped or overflowing its container; horizontal scroll on the page body
- a control with no label, or a label that does not say what it does
- a raw decimal where a formatted amount belongs (e.g. "0.500000000000000000")
- an identifier shown where a name was promised
- text overlapping, or a badge breaking a row's baseline
- an empty state that reads as a failure rather than as "nothing here yet"
- a disabled control with no explanation of why

An empty page because the account has no data yet is correct, not a defect —
only report it if it *looks* broken (an error-shaped box, missing copy, a
spinner that never resolved). Do not invent a severity score; that is not
yours to judge here. If a viewport is fine, do not report anything for it.
Call report_findings with an empty array if both viewports are fine.`;

const FINDINGS_TOOL = {
  name: 'report_findings',
  description: 'Report visual defects found in the supplied screenshots.',
  input_schema: {
    type: 'object',
    properties: {
      findings: {
        type: 'array',
        items: {
          type: 'object',
          properties: {
            viewport: { type: 'string', enum: ['mobile', 'desktop'] },
            what_is_wrong: { type: 'string' },
          },
          required: ['viewport', 'what_is_wrong'],
        },
      },
    },
    required: ['findings'],
  },
};

function loadManifest() {
  if (!fs.existsSync(MANIFEST_PATH)) return null;
  return JSON.parse(fs.readFileSync(MANIFEST_PATH, 'utf8'));
}

// One route per call, both viewports in the same request — half the calls of
// reviewing each screenshot alone, and the model can tell what changed
// between mobile and desktop rather than judging each in isolation.
function groupByRoute(manifest, errors) {
  const routes = new Map();
  for (const entry of manifest) {
    if (!entry.file) {
      // Capture failed — nothing to look at, but say so instead of letting
      // the route disappear from the report as if it had never been listed.
      errors.push({
        route: entry.route,
        viewport: entry.viewport,
        reason: entry.error ? `capture failed: ${entry.error}` : 'capture failed',
      });
      continue;
    }
    if (!routes.has(entry.route)) routes.set(entry.route, { path: entry.path, shots: [] });
    routes.get(entry.route).shots.push(entry);
  }
  return routes;
}

async function reviewRoute(apiKey, route, { path: routePath, shots }) {
  const content = [{ type: 'text', text: `Route: ${routePath}` }];
  for (const shot of shots) {
    const data = fs.readFileSync(path.join(VISUAL_DIR, shot.file)).toString('base64');
    content.push({ type: 'text', text: `Viewport: ${shot.viewport}` });
    content.push({ type: 'image', source: { type: 'base64', media_type: 'image/png', data } });
  }

  const resp = await fetch(API_URL, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-api-key': apiKey,
      'anthropic-version': '2023-06-01',
    },
    body: JSON.stringify({
      model: MODEL,
      max_tokens: 1024,
      system: RUBRIC,
      messages: [{ role: 'user', content }],
      tools: [FINDINGS_TOOL],
      tool_choice: { type: 'tool', name: 'report_findings' },
    }),
  });

  if (!resp.ok) {
    const body = await resp.text().catch(() => '');
    throw new Error(`${resp.status} ${resp.statusText}: ${body.slice(0, 500)}`);
  }

  const data = await resp.json();
  const toolUse = data.content?.find((b) => b.type === 'tool_use' && b.name === 'report_findings');
  if (!toolUse) throw new Error(`no report_findings tool call in response: ${JSON.stringify(data).slice(0, 500)}`);

  return (toolUse.input?.findings ?? []).map((f) => ({ route, path: routePath, ...f }));
}

// Errors are not findings, but they must never be invisible: a route that
// could not be captured or reviewed and a route that was reviewed and found
// clean must not render identically, or the only signal that the pipeline
// broke is a console line nobody watches on a nightly run.
function renderReport(findings, errors) {
  const lines = ['# Nightly visual review', ''];
  if (errors.length > 0) {
    lines.push(
      `**${errors.length} route(s) could not be reviewed — treat this run as incomplete, not clean:**`,
      '',
    );
    for (const e of errors) {
      const label = e.route ? `${e.route}${e.viewport && e.viewport !== 'n/a' ? ` (${e.viewport})` : ''}` : 'pipeline';
      lines.push(`- ${label}: ${e.reason}`);
    }
    lines.push('');
  }

  if (findings.length === 0) {
    lines.push(errors.length > 0 ? 'No findings among the routes that were reviewed.' : 'No issues found.');
    return lines.join('\n') + '\n';
  }

  const byRoute = new Map();
  for (const f of findings) {
    if (!byRoute.has(f.route)) byRoute.set(f.route, { path: f.path, items: [] });
    byRoute.get(f.route).items.push(f);
  }
  lines.push(`${findings.length} finding(s) across ${byRoute.size} route(s).`, '');
  for (const [route, { path: routePath, items }] of byRoute) {
    lines.push(`## ${routePath} (${route})`);
    for (const item of items) {
      lines.push(`- **${item.viewport}**: ${item.what_is_wrong}`);
    }
    lines.push('');
  }
  return lines.join('\n');
}

function writeOutputs(findings, errors) {
  fs.mkdirSync(VISUAL_DIR, { recursive: true });
  fs.writeFileSync(path.join(VISUAL_DIR, 'findings.json'), JSON.stringify({ findings, errors }, null, 2));
  const report = renderReport(findings, errors);
  fs.writeFileSync(path.join(VISUAL_DIR, 'report.md'), report);
  console.log(report);
}

async function main() {
  fs.mkdirSync(VISUAL_DIR, { recursive: true });

  const findings = [];
  const errors = [];

  const apiKey = process.env.ANTHROPIC_API_KEY;
  if (!apiKey) {
    // Deliberately not an error entry: normal CI runs this script with no
    // key and no manifest, and that path must stay silent. The scheduled
    // workflow always sets the secret, so if it is ever missing there the
    // pipeline itself is broken — the workflow's own "File findings" step
    // treats a missing findings.json as exactly that signal.
    console.log('ANTHROPIC_API_KEY unset — skipping visual review');
    return;
  }

  const manifest = loadManifest();
  if (!manifest || manifest.length === 0) {
    errors.push({ route: null, viewport: null, reason: `no manifest at ${MANIFEST_PATH} — review did not run` });
    writeOutputs(findings, errors);
    return;
  }

  const routes = groupByRoute(manifest, errors);
  if (routes.size === 0) {
    errors.push({ route: null, viewport: null, reason: 'every capture in the manifest failed — nothing to review' });
    writeOutputs(findings, errors);
    return;
  }

  for (const [route, group] of routes) {
    try {
      findings.push(...(await reviewRoute(apiKey, route, group)));
    } catch (err) {
      const reason = `review failed: ${err instanceof Error ? err.message : err}`;
      console.error(`${route}: ${reason}`);
      errors.push({ route, viewport: null, reason });
    }
  }

  writeOutputs(findings, errors);
}

main().catch((err) => {
  // Advisory tooling: log and exit 0 rather than failing a job that exists
  // to produce a report a human skims, not a gate anyone depends on being green.
  // Still write findings.json — an empty one would read as "reviewed, clean",
  // which is exactly the outcome a crash must not produce.
  const reason = `visual-review.mjs crashed: ${err instanceof Error ? err.stack : err}`;
  console.error(reason);
  try {
    writeOutputs([], [{ route: null, viewport: null, reason }]);
  } catch (writeErr) {
    console.error(`could not even write the failure report: ${writeErr instanceof Error ? writeErr.stack : writeErr}`);
  }
});
