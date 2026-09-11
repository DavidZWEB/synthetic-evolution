/** Browser run ownership, acknowledged delivery, and archive lifecycle regressions. */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import { writeRunUrl } from '../src/sim/seed-url.js';
import { parseArchive } from '../src/history/archive.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const params = {
  world: { max_agents: 32 },
  plants: { max_plants: 16 },
  chemo: { cells: [8, 8, 1] },
};

async function withPage(mode, run, { beforeLoad } = {}) {
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
    await beforeLoad?.(page);
    await page.goto(writeRunUrl(server.resolvedUrls.local[0], {
      seed: '42', founders: 32, params: JSON.stringify(params), brainInheritance: 'evolving',
    }));
    await page.getByRole('button', { name: 'step', exact: true }).waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll('header dt')]
      .find((node) => node.textContent === 'transport')?.nextElementSibling?.textContent !== '…');
    await run(page);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    await server.close();
  }
}

async function openCapture(page) {
  await page.getByRole('button', { name: 'history', exact: true }).click();
  const panel = page.getByRole('region', { name: 'Species history' });
  await panel.getByRole('checkbox').check();
  await page.getByRole('button', { name: 'reseed', exact: true }).click();
  await page.getByRole('button', { name: 'history (recording)', exact: true }).waitFor();
  await panel.getByText('recording in this tab', { exact: true }).waitFor();
  return panel;
}

async function download(page, button) {
  const downloading = page.waitForEvent('download');
  await button.click();
  const file = await downloading;
  return readFile(await file.path(), 'utf8');
}

for (const mode of ['development', 'transferable']) {
  test(`saved histories are per-run, round-trip prefixes, and never resume Worlds (${mode})`,
    { timeout: 60_000 }, async () => {
      await withPage(mode, async (page) => {
        assert.equal(await page.evaluate(() => crossOriginIsolated), mode !== 'transferable');
        let panel = await openCapture(page);
        await page.getByRole('button', { name: 'step', exact: true }).click();
        await page.waitForFunction(() => [...document.querySelectorAll('header dt')]
          .find((node) => node.textContent === 'tick')?.nextElementSibling?.textContent === '1');
        const text = await download(page, panel.getByRole('button', { name: 'export snapshot' }));
        const snapshot = await parseArchive(text);
        assert.equal(snapshot.header.data.provenance.seed, '42');
        assert.match(snapshot.header.data.provenance.source_revision, /-development$/);
        assert.deepEqual(snapshot.header.data.cohorts, ['evolving']);
        assert.equal(snapshot.completion.data.capture_end, 'snapshot');
        assert.equal(snapshot.completion.data.ticks, '1');
        assert.match(snapshot.completion.data.cohorts[0].final_state_hash, /^[0-9a-f]{16}$/);

        await page.getByRole('button', { name: 'reseed', exact: true }).click();
        await page.waitForFunction(() => document.querySelectorAll('.history-panel li').length === 2);
        await panel.getByRole('list').getByText('reseeded', { exact: true }).waitFor();
        const archiveIds = await panel.locator('.identity').allTextContents();
        assert.equal(new Set(archiveIds).size, 2);
        await panel.getByRole('button', { name: 'stop recording', exact: true }).click();
        await panel.getByText('stopped', { exact: true }).last().waitFor();

        await page.reload();
        await page.getByRole('button', { name: 'history', exact: true }).click();
        panel = page.getByRole('region', { name: 'Species history' });
        assert.equal(await panel.getByRole('checkbox').isChecked(), false);
        await page.waitForFunction(() => document.querySelectorAll('.history-panel li').length === 2);
        await panel.locator('input[type=file]').setInputFiles({
          name: 'snapshot.jsonl', mimeType: 'application/x-ndjson', buffer: Buffer.from(text),
        });
        await page.waitForFunction(() => document.querySelectorAll('.history-panel li').length === 3);
        await panel.getByText('History imported. No simulation was started or resumed.').waitFor();
        const tick = await page.locator('header').first().locator('dt').evaluateAll((nodes) =>
          nodes.find((node) => node.textContent === 'tick')?.nextElementSibling?.textContent);
        assert.equal(tick, '0');
        for (const width of [390, 320]) {
          await page.setViewportSize({ width, height: 844 });
          assert.equal(await page.evaluate(() =>
            document.documentElement.scrollWidth > innerWidth), false);
          assert.equal(await panel.evaluate((node) => node.scrollWidth > node.clientWidth), false);
        }

        await panel.getByRole('button', { name: 'delete', exact: true }).first().click();
        await page.waitForFunction(() => document.querySelectorAll('.history-panel li').length === 2);
      });
    });

  test(`history backpressure does not require snapshot consumption (${mode})`,
    { timeout: 60_000 }, async () => {
      await withPage(mode, async (page) => {
        const result = await page.evaluate(async (params) => {
          const { createSim } = await import('/src/sim/client.js');
          const sim = createSim({
            seed: '42', founders: 32, params: JSON.stringify(params),
            historyRunId: crypto.randomUUID(),
          });
          const batches = [];
          const errors = [];
          sim.on('historyBatch', (batch) => batches.push(batch));
          sim.on('error', (error) => errors.push(error));
          sim.on('historyError', (error) => errors.push(error));
          try {
            await new Promise((resolve) => sim.on('ready', resolve));
            const inspected = () => new Promise((resolve) => {
              const off = sim.on('inspection', (message) => { off(); resolve(message); });
              sim.inspect(0, 1, 1);
            });
            for (let i = 0; i < 4; i++) {
              sim.stepOnce(1);
              await inspected();
            }
            const firstCount = batches.length;
            const inspection = await inspected();
            const boundary = new Promise((resolve) => {
              const off = sim.on('historyBatch', (batch) => {
                if (batch.requestId === 1) { off(); resolve(batch); }
              });
            });
            sim.historyBoundary('snapshot', 1);
            sim.acknowledgeHistory(batches[0].batchId);
            const final = await boundary;
            return {
              firstCount, tick: JSON.parse(inspection.agent).tick, finalTick: final.tick,
              captureEnd: final.captureEnd, errors,
            };
          } finally {
            sim.destroy();
          }
        }, params);
        assert.equal(result.firstCount, 1);
        assert.equal(result.tick, '4');
        assert.equal(result.finalTick, '4');
        assert.equal(result.captureEnd, 'snapshot');
        assert.deepEqual(result.errors, []);
      });
    });

  test(`only successful retuning closes history at the old parameter boundary (${mode})`,
    { timeout: 60_000 }, async () => {
      await withPage(mode, async (page) => {
        const result = await page.evaluate(async (params) => {
          const { createSim } = await import('/src/sim/client.js');
          const sim = createSim({
            seed: '42', founders: 32, params: JSON.stringify(params),
            historyRunId: crypto.randomUUID(),
          });
          const batches = [];
          sim.on('historyBatch', (batch) => {
            batches.push(batch);
            sim.acknowledgeHistory(batch.batchId);
          });
          const once = (kind) => new Promise((resolve) => {
            const off = sim.on(kind, (message) => { off(); resolve(message); });
          });
          const hash = async () => {
            const value = once('hash');
            sim.requestHash();
            return (await value).value.padStart(16, '0');
          };
          try {
            const { run } = await once('ready');
            const normalized = JSON.parse(run.params);
            const before = await hash();
            const rejected = once('error');
            sim.setParams(JSON.stringify({
              ...normalized, species: { ...normalized.species, threshold: 0.25 },
            }));
            const rejection = await rejected;
            const afterRejection = await hash();
            const earlyClosures = batches.filter((batch) => batch.captureEnd).length;
            const closed = new Promise((resolve) => {
              const off = sim.on('historyBatch', (batch) => {
                if (batch.captureEnd) { off(); resolve(batch); }
              });
            });
            const accepted = once('params');
            sim.setParams(JSON.stringify({
              ...normalized, metabolism: { ...normalized.metabolism, base: 0.1 },
            }));
            const boundary = await closed;
            return {
              before, afterRejection, earlyClosures,
              acceptedBase: JSON.parse((await accepted).params).metabolism.base,
              rejection: rejection.context, boundary,
            };
          } finally {
            sim.destroy();
          }
        }, params);
        assert.equal(result.rejection, 'set_params');
        assert.equal(result.before, result.afterRejection);
        assert.equal(result.acceptedBase, 0.1);
        assert.equal(result.earlyClosures, 0);
        assert.equal(result.boundary.captureEnd, 'params_changed');
        assert.equal(result.boundary.stateHash, result.before);
        assert.equal(result.boundary.tick, '0');
      });
    });
}

test('quota errors leave an exportable incomplete prefix while the World keeps stepping',
  { timeout: 60_000 }, async () => {
    await withPage('development', async (page) => {
      const panel = await openCapture(page);
      const snapshot = await download(page, panel.getByRole('button', { name: 'export snapshot' }));
      const before = await parseArchive(snapshot);
      await page.evaluate(() => {
        const put = IDBObjectStore.prototype.put;
        globalThis.restoreHistoryWrites = () => { IDBObjectStore.prototype.put = put; };
        IDBObjectStore.prototype.put = function () {
          throw new DOMException('test quota exhausted', 'QuotaExceededError');
        };
      });
      await panel.getByRole('button', { name: 'stop recording', exact: true }).click();
      await page.getByText('history incomplete', { exact: true }).waitFor();
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() => [...document.querySelectorAll('header dt')]
        .find((node) => node.textContent === 'tick')?.nextElementSibling?.textContent === '1');
      await page.evaluate(() => globalThis.restoreHistoryWrites());
      const exported = await download(page, panel.getByRole('button', { name: 'export', exact: true }));
      const after = await parseArchive(exported);
      assert.equal(after.completion.data.capture_end, 'unfinalized');
      assert.equal(after.completion.data.cohorts[0].history_complete, false);
      assert.equal(after.completion.data.ticks, before.completion.data.ticks);
      assert.deepEqual(after.rows, before.rows);
    });
  });

test('reseed waits for an in-progress snapshot before finalizing the old capture',
  { timeout: 60_000 }, async () => {
    await withPage('development', async (page) => {
      const panel = await openCapture(page);
      const downloaded = page.waitForEvent('download');
      await panel.getByRole('button', { name: 'export snapshot' }).click();
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      const file = await downloaded;
      const snapshot = await parseArchive(await readFile(await file.path(), 'utf8'));
      assert.equal(snapshot.completion.data.capture_end, 'snapshot');
      await page.waitForFunction(() => document.querySelectorAll('.history-panel li').length === 2);
      await panel.getByRole('list').getByText('reseeded', { exact: true }).waitFor();
      await panel.getByText('recording in this tab', { exact: true }).waitFor();
      assert.doesNotMatch(await panel.getByRole('list').innerText(), /incomplete|unfinalized/);
    }, {
      beforeLoad: (page) => page.addInitScript(() => {
        const OriginalWorker = globalThis.Worker;
        globalThis.Worker = class extends OriginalWorker {
          set onmessage(callback) {
            super.onmessage = (event) => {
              if (event.data.kind === 'historyBatch' && event.data.captureEnd === 'snapshot') {
                setTimeout(() => callback(event), 300);
              } else {
                callback(event);
              }
            };
          }
        };
      }),
    });
  });
