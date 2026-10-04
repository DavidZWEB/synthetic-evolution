/** Real worker, paused rendering, and species-selection lifecycle regressions. */
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import { writeRunUrl } from '../src/sim/seed-url.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const founders = 128;

async function withPage(mode, run, params = {}, { seed = '42', runFounders = founders } = {}) {
  const server = await createServer({
    root, mode, logLevel: 'silent', server: { host: '127.0.0.1', strictPort: false },
  });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript(() => {
      const RealWorker = globalThis.Worker;
      globalThis.speciesTestWorkers = [];
      globalThis.Worker = class extends RealWorker {
        constructor(...args) { super(...args); globalThis.speciesTestWorkers.push(this); }
        set onmessage(callback) {
          super.onmessage = (event) => {
            if (event.data.kind === 'species' && globalThis.delayNextSpecies) {
              globalThis.delayNextSpecies = false;
              setTimeout(() => {
                globalThis.delayedSpeciesDelivered = true;
                callback(event);
              }, 400);
            } else callback(event);
          };
        }
      };
    });
    await page.goto(writeRunUrl(server.resolvedUrls.local[0], {
      seed, founders: runFounders, brainInheritance: 'evolving',
      params: JSON.stringify({
        world: { max_agents: 128 }, plants: { max_plants: 64 }, chemo: { cells: [8, 8, 1] },
        species: { capacity: 4, threshold: 1e-12 }, brain: { connections_per_target: 1 },
        ...params,
      }),
    }));
    await page.waitForFunction((expected) => [...document.querySelectorAll('header dt')]
      .find((node) => node.textContent === 'transport')?.nextElementSibling.textContent === expected,
    mode === 'transferable' ? 'transferable' : 'shared');
    assert.equal(await page.evaluate(() => crossOriginIsolated), mode !== 'transferable');
    await run(page);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    await server.close();
  }
}

async function openSpecies(page) {
  await page.getByRole('button', { name: 'species', exact: true }).click();
  const panel = page.getByRole('region', { name: 'Live species' });
  await panel.locator('.sample-time').waitFor();
  return panel;
}

async function firstFounder(page, index = 0) {
  const box = await page.locator('canvas').first().boundingBox();
  const radius = 400 * Math.sqrt((index + 0.5) / founders);
  const angle = index * 2.3999632;
  const scale = Math.min(box.width, box.height) / 1000;
  return {
    x: box.x + box.width / 2 + radius * Math.cos(angle) * scale,
    y: box.y + box.height / 2 - radius * Math.sin(angle) * scale,
  };
}

async function hash(page) {
  return page.evaluate(() => new Promise((resolve) => {
    const worker = globalThis.speciesTestWorkers.at(-1);
    const receive = (event) => {
      if (event.data.kind !== 'hash') return;
      worker.removeEventListener('message', receive);
      resolve(event.data.value);
    };
    worker.addEventListener('message', receive);
    worker.postMessage({ kind: 'hash' });
  }));
}

async function paintedAgent(page, point) {
  await page.evaluate(() => new Promise((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(resolve))));
  return page.screenshot({ clip: { x: Math.floor(point.x) - 4, y: Math.floor(point.y) - 4, width: 9, height: 9 } });
}

for (const mode of ['development', 'transferable']) {
  test(`species colors repaint paused frames without changing World state or picking (${mode})`, async () => {
    await withPage(mode, async (page) => {
      const panel = await openSpecies(page);
      assert.equal(await panel.getByRole('combobox', { name: 'color by' }).inputValue(), 'signature');
      assert.equal(await panel.locator('.sample-time').innerText(), 'as of tick 0 · 128 agents');
      assert.equal(await panel.getByRole('button', { name: /^Highlight species / }).count(), 4);
      const unclassified = panel.getByRole('button', { name: 'Highlight unclassified agents, 124 agents' });
      assert.equal(await unclassified.count(), 1);
      const beforeHash = await hash(page);
      const point = await firstFounder(page);
      const original = await paintedAgent(page, point);
      await panel.getByRole('combobox', { name: 'color by' }).selectOption('species');
      const colored = await paintedAgent(page, point);
      assert.notDeepEqual(colored, original, 'species mode did not repaint the paused canvas');
      await panel.getByRole('button', { name: 'Highlight species 1, 1 agents', exact: true }).click();
      const dimmed = await paintedAgent(page, point);
      assert.notDeepEqual(dimmed, colored, 'unselected species were not dimmed');
      await panel.getByRole('button', { name: 'clear species highlight' }).click();
      assert.deepEqual(await paintedAgent(page, point), colored);
      await panel.getByRole('combobox', { name: 'color by' }).selectOption('signature');
      assert.deepEqual(await paintedAgent(page, point), original, 'genetic signature display was not restored');
      assert.equal(await hash(page), beforeHash, 'display changes or queries changed World state');
      await page.mouse.click(point.x, point.y);
      const inspector = page.getByRole('complementary', { name: 'Inspector for agent 0' });
      await inspector.waitFor();
      assert.equal(await panel.count(), 0, 'species panel obscures picked-agent inspection');
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() => document.querySelector('aside dt')?.nextElementSibling.textContent === '1');
      const reopened = await openSpecies(page);
      assert.equal(await page.getByRole('complementary').count(), 0);
      await page.waitForFunction(() => document.querySelector('.species-panel .sample-time')
        ?.textContent.includes('tick 1'));
      assert.match(await reopened.locator('.sample-time').innerText(), /tick 1/);
      for (const width of [390, 320]) {
        await page.setViewportSize({ width, height: 844 });
        const layout = await reopened.evaluate((node) => ({
          document: document.documentElement.scrollWidth, viewport: innerWidth,
          scroll: node.scrollWidth, client: node.clientWidth,
        }));
        assert.equal(layout.document, layout.viewport);
        assert.equal(layout.scroll, layout.client);
      }
    });
  });

  test(`species retirement clears highlight while keeping unclassified data distinct (${mode})`, async () => {
    await withPage(mode, async (page) => {
      const panel = await openSpecies(page);
      await panel.getByRole('button', { name: 'Highlight species 0, 1 agents', exact: true }).click();
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await panel.getByText('Species #0 is no longer active.', { exact: true }).waitFor();
      assert.equal(await panel.getByRole('button', { name: 'clear species highlight' }).isDisabled(), true);
      assert.equal(await panel.getByRole('button', { name: /^Highlight species / }).count(), 0);
      assert.equal(await panel.getByRole('button', { name: 'Highlight unclassified agents, 0 agents' }).isDisabled(), true);
    }, { metabolism: { base: 1_000_000 } });
  });

  test(`context restoration preserves species colors and highlight on an unchanged frame (${mode})`, async () => {
    await withPage(mode, async (page) => {
      const panel = await openSpecies(page);
      await panel.getByRole('combobox', { name: 'color by' }).selectOption('species');
      await panel.getByRole('button', { name: 'Highlight species 0, 1 agents', exact: true }).click();
      const point = await firstFounder(page, 1);
      const before = await paintedAgent(page, point);
      const beforeHash = await hash(page);
      await page.evaluate(async () => {
        const canvas = document.querySelector('canvas');
        const extension = canvas.getContext('webgl2').getExtension('WEBGL_lose_context');
        if (!extension) throw new Error('context-loss test extension is unavailable');
        const lost = new Promise((resolve) => canvas.addEventListener('webglcontextlost', resolve, { once: true }));
        extension.loseContext();
        await lost;
        await new Promise((resolve) => setTimeout(resolve, 50));
        const restored = new Promise((resolve) =>
          canvas.addEventListener('webglcontextrestored', resolve, { once: true }));
        extension.restoreContext();
        await restored;
      });
      assert.deepEqual(await paintedAgent(page, point), before);
      assert.equal(await hash(page), beforeHash);
      assert.equal(await page.getByRole('alert').count(), 0);
    });
  });
}

test('reseed rejects delayed old-world species replies and resets selection without resetting color mode', async () => {
  await withPage('development', async (page) => {
    const panel = await openSpecies(page);
    await panel.getByRole('combobox', { name: 'color by' }).selectOption('species');
    await panel.getByRole('button', { name: 'Highlight species 0, 1 agents', exact: true }).click();
    await panel.getByRole('button', { name: 'Close species browser' }).click();
    await page.evaluate(() => { globalThis.delayNextSpecies = true; });
    await page.getByRole('button', { name: 'species', exact: true }).click();
    await page.waitForFunction(() => globalThis.delayNextSpecies === false);
    await page.getByRole('spinbutton', { name: 'founders', exact: true }).fill('16');
    await page.getByRole('button', { name: 'reseed', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.species-panel .sample-time')
      ?.textContent.includes('16 agents'));
    await page.waitForFunction(() => globalThis.delayedSpeciesDelivered === true);
    assert.equal(await panel.locator('.sample-time').innerText(), 'as of tick 0 · 16 agents');
    assert.equal(await panel.getByRole('combobox', { name: 'color by' }).inputValue(), 'species');
    assert.equal(await panel.getByRole('button', { name: 'clear species highlight' }).isDisabled(), true);
  });
});

test('genome complexity matches the native reference at the same completed tick', async () => {
  // shells/shared/complexity_case.rs: the reference native metrics and WASM tests share.
  const structural = JSON.parse(readFileSync(
    new URL('../../shells/native/tests/fixtures/structural.json', import.meta.url), 'utf8'));
  await withPage('development', async (page) => {
    const panel = await openSpecies(page);
    for (let tick = 1; tick <= 40; tick++) {
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction((expected) => [...document.querySelectorAll('header dt')]
        .find((node) => node.textContent === 'tick')?.nextElementSibling?.textContent === expected,
      String(tick));
    }
    await page.waitForFunction(() => document.querySelector('.species-panel .sample-time')
      ?.textContent.startsWith('as of tick 40 '));
    const table = panel.getByRole('table', { name: 'Genome complexity' });
    const rows = await table.locator('tbody tr').evaluateAll((nodes) => nodes.map((row) =>
      [...row.children].map((cell) => cell.textContent)));
    // Read the shared reference itself, so this cannot drift from native and WASM.
    const source = readFileSync(
      new URL('../../shells/shared/complexity_case.rs', import.meta.url), 'utf8');
    const expected = JSON.parse([...source.matchAll(/r#"(.*?)"#/g)].map((m) => m[1]).join(''));
    const row = (label, d) => [label, String(d.median), `${d.p25}–${d.p75}`, `${d.min}–${d.max}`,
      d.mean.toFixed(1)];
    assert.deepEqual(rows, [
      row('all genes', expected.genome_genes),
      row('neurons', expected.neurons),
      row('connections', expected.connections),
      row('enabled connections', expected.enabled_connections),
      row('wired hidden neurons', expected.wiring.wired_hidden_neurons),
      row('wired sensors', expected.wiring.wired_sensors),
      row('driven effectors', expected.wiring.driven_effectors),
    ]);
  }, { ...structural, species: { capacity: 256, threshold: 0.5 } }, { seed: '7', runFounders: 8 });
});

test('classification-disabled worlds remain explicitly unclassified in the species browser', async () => {
  await withPage('development', async (page) => {
    const panel = await openSpecies(page);
    assert.equal(await panel.getByRole('button', { name: /^Highlight species / }).count(), 0);
    const unclassified = panel.getByRole('button', { name: 'Highlight unclassified agents, 128 agents' });
    await unclassified.click();
    assert.equal(await unclassified.getAttribute('aria-pressed'), 'true');
    assert.match(await panel.innerText(), /No classified species are active/);
  }, { species: { capacity: 0 } });
});
