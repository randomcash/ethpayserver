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
