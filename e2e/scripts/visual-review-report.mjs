/**
 * The pure shaping logic behind visual-review.mjs's report: which manifest
 * entries are findings vs errors, and how the two render. Split out so it can
 * be unit-tested without importing visual-review.mjs itself, which runs
 * main() (an API call) as a module-load side effect.
 */

// One route per call, both viewports in the same request — half the calls of
// reviewing each screenshot alone, and the model can tell what changed
// between mobile and desktop rather than judging each in isolation.
export function groupByRoute(manifest, errors) {
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

// Errors are not findings, but they must never be invisible: a route that
// could not be captured or reviewed and a route that was reviewed and found
// clean must not render identically, or the only signal that the pipeline
// broke is a console line nobody watches on a nightly run.
export function renderReport(findings, errors) {
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

// visual-review.mjs's top-level main().catch() must turn an unexpected throw
// into the same shape writeOutputs() always produces — an empty findings
// array and a non-empty errors array — never an empty findings.json, which
// would render identically to a clean run. Pulled out so that invariant is
// checkable without actually crashing main() end to end.
export function crashOutputs(err) {
  const reason = `visual-review.mjs crashed: ${err instanceof Error ? err.stack : err}`;
  return { findings: [], errors: [{ route: null, viewport: null, reason }] };
}

// The redacted counterpart to renderReport(), for the one output this
// pipeline actually publishes: a public, durable GitHub issue. Deliberately
// never touches `what_is_wrong` or a screenshot path — see the scheduled
// workflow's header comment for why that text cannot leave this checkout.
// Kept next to renderReport so the issue-filing step has a tested function to
// call instead of re-deriving the route/count grouping on its own; that
// duplication is what let a route silently disappear from a public report
// undetected before this function existed.
// Returns null for a clean run with nothing to report, so the caller can
// skip opening or commenting on an issue at all.
export function renderPublicSummary(findings, errors, runUrl) {
  if (findings.length === 0 && errors.length === 0) return null;

  const lines = ['# Nightly visual review', ''];
  if (findings.length > 0) {
    const byRoute = new Map();
    for (const f of findings) byRoute.set(f.route, (byRoute.get(f.route) ?? 0) + 1);
    lines.push(`${findings.length} finding(s) across ${byRoute.size} route(s):`, '');
    for (const [route, count] of byRoute) lines.push(`- \`${route}\`: ${count}`);
    lines.push('');
  }
  if (errors.length > 0) {
    lines.push(`${errors.length} route(s) could not be reviewed — treat this run as incomplete, not clean:`, '');
    for (const e of errors) {
      const label = e.route ? `${e.route}${e.viewport && e.viewport !== 'n/a' ? ` (${e.viewport})` : ''}` : 'pipeline';
      lines.push(`- ${label}: ${e.reason}`);
    }
    lines.push('');
  }
  lines.push(
    `Run: ${runUrl}`,
    '',
    'Per-finding detail (what is wrong, per route and viewport) and the',
    'screenshots are not published anywhere from this run — advisory',
    'findings describe live defects in a production payment processor,',
    'and this repo has no private place to put them yet (an artifact or',
    'a job log here is exactly as public as this issue). That needs a',
    'private receiver this repo alone cannot add; see the workflow file.',
    '',
    'This is advisory — a model judging layout will produce false positives.',
  );
  return lines.join('\n');
}
