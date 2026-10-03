/** Species-origin graph and representative comparison in a real browser. */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import { writeRunUrl } from '../src/sim/seed-url.js';

const root = fileURLToPath(new URL('../', import.meta.url));
// A tiny threshold makes every founder its own species, so origins have representatives.
const params = {
  world: { max_agents: 32 }, plants: { max_plants: 16 }, chemo: { cells: [8, 8, 1] },
  species: { capacity: 32, threshold: 1e-6 },
};

async function withPage(run) {
  const server = await createServer({
    root, mode: 'development', logLevel: 'silent', server: { host: '127.0.0.1', strictPort: false },
  });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(writeRunUrl(server.resolvedUrls.local[0], {
      seed: '42', founders: 32, params: JSON.stringify(params), brainInheritance: 'evolving',
    }));
    await page.getByRole('button', { name: 'step', exact: true }).waitFor();
    await run(page);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    await server.close();
  }
}

async function openLineage(page) {
  const history = page.getByRole('region', { name: 'Species history' });
  await history.getByRole('button', { name: /^lineage for archive / }).first().click();
  const panel = page.getByRole('region', { name: 'Species lineage' });
  await panel.waitFor();
  return panel;
}

test('recorded origins draw a layered tree and two representatives compare', { timeout: 90_000 }, async () => {
  await withPage(async (page) => {
    await page.getByRole('button', { name: 'history', exact: true }).click();
    const history = page.getByRole('region', { name: 'Species history' });
    await history.getByRole('checkbox', { name: 'record the next new / reseeded run' }).check();
    await history.getByRole('checkbox', { name: 'include representative genomes' }).check();
    await page.getByRole('button', { name: 'reseed', exact: true }).click();
    await history.getByText('recording in this tab', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'step', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.history-panel li'));

    const panel = await openLineage(page);
    const species = panel.getByRole('button', { name: /^Species \d+, origin tick/ });
    assert.ok(await species.count() > 1, 'founders form distinct species');
    await species.nth(0).click();
    await species.nth(1).focus();
    await page.keyboard.press('Enter');
    assert.equal(await species.nth(1).getAttribute('aria-pressed'), 'true', 'keyboard selection');
    await panel.getByText(/^Species #0 vs #1: distance/).waitFor();
    const distance = Number((await panel.getByText(/^Species #0 vs #1: distance/).innerText())
      .match(/distance ([0-9.]+)/)[1]);
    assert.ok(distance >= 1e-6, 'distinct species are at least the threshold apart');
    assert.equal(await panel.getByRole('table', { name: 'Gene alignment' }).locator('tbody tr').count(), 4);

    const before = await species.count();
    await panel.getByRole('checkbox', { name: 'hide extinct lineages without descendants' }).check();
    assert.equal(await species.count(), before, 'nothing has gone extinct yet');

    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      assert.equal(await panel.evaluate((node) => node.scrollWidth > node.clientWidth), false);
    }
    await panel.getByRole('button', { name: 'Close species lineage' }).click();
    assert.equal(await panel.count(), 0);
  });
});

test('archives without genomes still show lineage and say why genomes are missing', { timeout: 90_000 }, async () => {
  const native = await readFile(new URL('../../shells/native/tests/fixtures/saved-run-native.sevrun', import.meta.url));
  await withPage(async (page) => {
    await page.getByRole('button', { name: 'history', exact: true }).click();
    const history = page.getByRole('region', { name: 'Species history' });
    await history.locator('input[type=file]:not([disabled])').waitFor({ state: 'attached' });
    await history.locator('input[type=file]').setInputFiles({
      name: 'native.sevrun', mimeType: 'application/octet-stream', buffer: native,
    });
    await history.getByText('Loaded saved run at tick 60, paused.').waitFor();
    await page.getByRole('button', { name: 'history', exact: true }).click();
    await page.getByRole('button', { name: 'history', exact: true }).click();
    const panel = await openLineage(page);
    assert.deepEqual(await panel.getByRole('combobox', { name: 'cohort' }).locator('option').allTextContents(),
      ['evolving', 'random_control']);
    const species = panel.getByRole('button', { name: /^Species \d+, origin tick/ });
    await species.nth(0).click();
    await species.nth(1).click();
    await panel.getByText(/representative this archive did not record genomes/).first().waitFor();
    await panel.getByRole('combobox', { name: 'cohort' }).selectOption('random_control');
    assert.equal(await panel.getByRole('button', { pressed: true }).count(), 0, 'switching cohort clears selection');
  });
});
