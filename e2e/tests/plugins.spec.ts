/**
 * The plugin system, end to end: install a module, and get a page out of it.
 *
 * Nothing in this suite covered plugins at all, and the gap was not academic.
 * Three separate pieces of that system shipped with passing unit tests and no
 * caller - the page route with an empty renderer registry, the declared-route
 * mount, and the reconciliation read - and in every case the unit tests were
 * green because a unit test cannot tell "this works" from "this is never
 * reached". These go through the HTTP surface a client actually uses.
 *
 * The plugin installed here is a fixture, not the real one: the real plugin
 * is private, and a public suite that needed it could not run. What this
 * proves is the machinery - install, register, resolve, render, authorise -
 * which is the part that lives in this repository. What the machinery is
 * *used for* is tested where that lives.
 */
import { test, expect } from '@playwright/test';

import { api, ApiError } from '../fixtures/api';
import { createUserWithApiKey, resetDatabase } from '../fixtures/db';
import {
  FIXTURE_ADMIN_PAGE_PATH,
  FIXTURE_PAGE_PATH,
  FIXTURE_PAGE_TEXT,
  FIXTURE_PLUGIN_ID,
  FIXTURE_PLUGIN_SLUG,
  FIXTURE_PLUGIN_WASM_BASE64,
  SECOND_MANIFEST_TOML,
  SECOND_PLUGIN_ID,
  SECOND_PLUGIN_SLUG,
} from '../fixtures/plugin-fixture';

interface PluginPage {
  path: string;
  label: string;
}
interface PluginListing {
  plugins: { id: string; slug: string; pages: PluginPage[] }[];
}
interface PageElement {
  type: string;
  title?: string;
  text?: string;
  children?: PageElement[];
  fields?: { label: string; value: string }[];
}

/** Every `text` in a page tree, flattened, so an assertion needs no path. */
function texts(element: PageElement): string[] {
  const here = element.text ? [element.text] : [];
  const kids = (element.children ?? []).flatMap(texts);
  return [...here, ...kids];
}

let admin: { userId: string; apiKey: string };
let merchant: { userId: string; apiKey: string };

test.beforeAll(async () => {
  // Deliberately NOT `resetDatabase()`: it would delete the seeded fixture's
  // install row, and the server has already loaded that plugin - leaving a
  // host holding a plugin the database has never heard of, which is a state
  // no deploy produces and no test should invent.
  admin = await createUserWithApiKey('server_admin');
  merchant = await createUserWithApiKey('user');
});

test.describe('a loaded plugin', () => {
  /**
   * These run against the plugin seeded before the server booted, so it is
   * genuinely loaded and genuinely rendering. Installing one through the API
   * and asking it for a page would test a plugin that is not running - the
   * host instantiates at boot - and the only honest outcome would be a skip.
   * A suite whose meaningful tests skip is green having tested nothing.
   */
  test('navigation lists it by slug, not by id', async () => {
    const listing = await api<PluginListing>('/plugins', { token: admin.apiKey });
    const plugin = listing.plugins.find((p) => p.id === FIXTURE_PLUGIN_ID);

    expect(plugin?.slug).toBe(FIXTURE_PLUGIN_SLUG);
    // The id is reverse-DNS so it can key a schema and an artifact safely.
    // It is exactly the wrong thing to put in a URL a person reads, which is
    // what the slug exists for.
    expect(plugin?.slug).not.toContain('.');
  });

  test('a page comes back from the plugin itself', async () => {
    const page = await api<PageElement>(
      `/plugins/${FIXTURE_PLUGIN_SLUG}/pages/${FIXTURE_PAGE_PATH}`,
      { token: merchant.apiKey },
    );

    expect(page.type).toBe('section');
    expect(texts(page)).toContain(FIXTURE_PAGE_TEXT);
  });

  test('the id still addresses the same page as the slug', async () => {
    // Admin surfaces, log lines and older clients all carry ids. Breaking
    // them to make a URL pretty would be a poor trade, so both resolve.
    const bySlug = await api<PageElement>(
      `/plugins/${FIXTURE_PLUGIN_SLUG}/pages/${FIXTURE_PAGE_PATH}`,
      { token: merchant.apiKey },
    );
    const byId = await api<PageElement>(
      `/plugins/${FIXTURE_PLUGIN_ID}/pages/${FIXTURE_PAGE_PATH}`,
      { token: merchant.apiKey },
    );
    expect(byId).toEqual(bySlug);
  });
});

test.describe('who may see what', () => {
  test('an admin-only page is not offered to a merchant', async () => {
    const asMerchant = await api<PluginListing>('/plugins', { token: merchant.apiKey });
    const asAdmin = await api<PluginListing>('/plugins', { token: admin.apiKey });

    const paths = (l: PluginListing) =>
      l.plugins.find((p) => p.id === FIXTURE_PLUGIN_ID)?.pages.map((x) => x.path) ?? [];

    expect(paths(asMerchant)).not.toContain(FIXTURE_ADMIN_PAGE_PATH);
    expect(paths(asAdmin)).toContain(FIXTURE_ADMIN_PAGE_PATH);
  });

  test('an unauthenticated caller gets nothing', async () => {
    await expect(api('/plugins')).rejects.toMatchObject({ status: 401 });
    await expect(
      api(`/plugins/${FIXTURE_PLUGIN_SLUG}/pages/${FIXTURE_PAGE_PATH}`),
    ).rejects.toMatchObject({ status: 401 });
  });
});

test.describe('what is refused', () => {
  const refused = async (path: string) => {
    try {
      await api(path, { token: admin.apiKey });
      return 0;
    } catch (e) {
      return e instanceof ApiError ? e.status : -1;
    }
  };

  test('an unknown page, plugin or malformed slug is a 404', async () => {
    expect(await refused(`/plugins/${FIXTURE_PLUGIN_SLUG}/pages/nosuchpage`)).toBe(404);
    expect(await refused(`/plugins/nosuchplugin/pages/${FIXTURE_PAGE_PATH}`)).toBe(404);
    // Uppercase is not the same slug: two that differ only by case would be
    // indistinguishable wherever something compares them case-insensitively.
    expect(
      await refused(`/plugins/${FIXTURE_PLUGIN_SLUG.toUpperCase()}/pages/${FIXTURE_PAGE_PATH}`),
    ).toBe(404);
  });
});

test.describe('installing one while the server runs', () => {
  /**
   * The other half of the lifecycle. The seeded fixture proves a loaded
   * plugin renders; this proves what *installing* does - and, just as
   * importantly, what it does not do. The host instantiates at boot, so an
   * install records a plugin without running it, and navigation must not
   * offer a page that would 404 on arrival.
   */
  test('is recorded, but not offered until it is loaded', async () => {
    await api('/admin/plugins', {
      method: 'POST',
      token: admin.apiKey,
      body: {
        manifest_toml: SECOND_MANIFEST_TOML,
        wasm_base64: FIXTURE_PLUGIN_WASM_BASE64,
        migrations: {},
      },
    });

    const installed = await api<{ plugins: { id: string; loaded: boolean }[] }>(
      '/admin/plugins',
      { token: admin.apiKey },
    );
    const row = installed.plugins.find((p) => p.id === SECOND_PLUGIN_ID);
    expect(row, 'the install must be recorded').toBeTruthy();
    expect(row?.loaded, 'nothing is compiled into a live host').toBe(false);

    const listing = await api<PluginListing>('/plugins', { token: admin.apiKey });
    expect(
      listing.plugins.map((p) => p.slug),
      'an unloaded plugin must not be offered — its page would 404 on arrival',
    ).not.toContain(SECOND_PLUGIN_SLUG);
  });
});
