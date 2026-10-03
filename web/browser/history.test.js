/** Browser run ownership, acknowledged delivery, and archive lifecycle regressions. */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import { writeRunUrl } from '../src/sim/seed-url.js';
import { encodeArchive, parseArchive } from '../src/history/archive.js';
import { fixtureArchive } from '../src/history/fixtures.js';

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
  await panel.getByRole('checkbox', { name: 'record the next new / reseeded run' }).check();
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

test('history file import rejects malformed UTF-8 and a leading BOM without normalizing bytes', async () => {
  const archive = fixtureArchive();
  archive.header.data.provenance.source_revision = 'revision~';
  archive.completion.data.provenance.source_revision = 'revision~';
  const valid = Buffer.from(encodeArchive(archive));
  const malformed = Buffer.from(valid);
  for (let i = 0; i < malformed.length; i++) {
    if (malformed[i] === 0x7e) malformed[i] = 0xff;
  }
  await withPage('development', async (page) => {
    await page.getByRole('button', { name: 'history', exact: true }).click();
    const panel = page.getByRole('region', { name: 'Species history' });
    for (const bytes of [malformed, Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), valid])]) {
      await panel.locator('input[type=file]').setInputFiles({
        name: 'invalid.jsonl', mimeType: 'application/x-ndjson', buffer: bytes,
      });
      await panel.getByRole('status').waitFor();
      assert.equal(await panel.getByRole('listitem').count(), 0, 'invalid bytes became a saved archive');
      assert.doesNotMatch(await panel.getByRole('status').innerText(), /History imported/);
    }
    const replacementCharacter = Buffer.from(valid.toString('utf8').replaceAll('revision~', 'revision\uFFFD'));
    await panel.locator('input[type=file]').setInputFiles({
      name: 'valid.jsonl', mimeType: 'application/x-ndjson', buffer: replacementCharacter,
    });
    await panel.getByText('History imported. No simulation was started or resumed.').waitFor();
    assert.equal(await panel.getByRole('listitem').count(), 1);
  });
});

test('archive action accessible names identify distinct archives even for the same seed', async () => {
  await withPage('development', async (page) => {
    const panel = await openCapture(page);
    await page.getByRole('button', { name: 'reseed', exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll('.history-panel li').length === 2);
    for (const row of await panel.getByRole('listitem').all()) {
      const id = await row.locator('.identity').getAttribute('title');
      const visibleExport = await row.locator('.actions button').first().innerText();
      assert.equal(await row.getByRole('button', {
        name: `${visibleExport} for archive ${id} (seed 42)`, exact: true,
      }).count(), 1);
      assert.equal(await row.getByRole('button', {
        name: `delete archive ${id} (seed 42)`, exact: true,
      }).count(), 1);
    }
  });
});

for (const failureKind of ['worker', 'renderer']) {
  test(`fatal ${failureKind} failure reports the owning history session as incomplete`, async () => {
    await withPage('development', async (page) => {
      let panel;
      if (failureKind === 'worker') {
        panel = await openCapture(page);
        await page.evaluate(() => globalThis.historyReviewWorkers.at(-1).onerror({
          preventDefault() {}, message: 'forced history worker failure',
        }));
        await panel.getByRole('list').getByText('capture_error', { exact: true }).waitFor();
      } else {
        await page.getByRole('button', { name: 'history', exact: true }).click();
        panel = page.getByRole('region', { name: 'Species history' });
        await panel.getByRole('checkbox', { name: 'record the next new / reseeded run' }).check();
        await page.evaluate(() => { globalThis.failHistoryRenderer = true; });
        await page.getByRole('button', { name: 'reseed', exact: true }).click();
      }
      await page.getByRole('alert').waitFor();
      assert.equal(await panel.locator('p strong').first().innerText(), 'incomplete');
      assert.equal(await page.getByText('history incomplete', { exact: true }).count(), 1);
      await page.evaluate(() => { globalThis.failHistoryRenderer = false; });
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await page.getByRole('button', { name: 'history (recording)', exact: true }).waitFor();
      await panel.getByText('recording in this tab', { exact: true }).waitFor();
      assert.equal(await panel.locator('p strong').first().innerText(), 'recording');
    }, {
      beforeLoad: (page) => page.addInitScript(() => {
        const RealWorker = globalThis.Worker;
        globalThis.historyReviewWorkers = [];
        globalThis.Worker = class extends RealWorker {
          constructor(...args) {
            super(...args);
            globalThis.historyReviewWorkers.push(this);
          }
        };
        const getContext = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (kind, ...args) {
          if (kind === 'webgl2' && globalThis.failHistoryRenderer) {
            throw new Error('forced history renderer initialization failure');
          }
          return getContext.call(this, kind, ...args);
        };
      }),
    });
  });
}

test('representative genomes are opt-in, archived at origin, and re-import with WASM validation',
  { timeout: 60_000 }, async () => {
    await withPage('development', async (page) => {
      await page.getByRole('button', { name: 'history', exact: true }).click();
      const panel = page.getByRole('region', { name: 'Species history' });
      const include = panel.getByRole('checkbox', { name: 'include representative genomes' });
      assert.equal(await include.isDisabled(), true, 'genomes require recording');
      await panel.getByRole('checkbox', { name: 'record the next new / reseeded run' }).check();
      await include.check();
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await panel.getByText('recording in this tab', { exact: true }).waitFor();
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() => [...document.querySelectorAll('header dt')]
        .find((node) => node.textContent === 'tick')?.nextElementSibling?.textContent === '1');
      const text = await download(page, panel.getByRole('button', { name: 'export snapshot' }));
      const snapshot = await parseArchive(text);
      assert.equal(snapshot.header.data.schema_version, 3);
      assert.equal(snapshot.header.data.representative_genes, 65_536);
      const origins = snapshot.rows.filter((row) => row.data.event?.kind === 'species_origin');
      assert.ok(origins.length > 0, 'founders originated species');
      for (const row of origins) {
        assert.equal(row.data.representative.status, 'recorded');
        assert.ok(row.data.representative.genes.length > 0);
      }
      const counts = snapshot.completion.data.cohorts[0].counts;
      assert.equal(counts.representatives, String(origins.length));
      assert.equal(counts.unavailable_representatives, '0');

      await panel.getByRole('button', { name: 'stop recording', exact: true }).click();
      await panel.getByText('stopped', { exact: true }).last().waitFor();
      await panel.locator('input[type=file]').setInputFiles({
        name: 'representatives.jsonl', mimeType: 'application/x-ndjson', buffer: Buffer.from(text),
      });
      await panel.getByText('History imported. No simulation was started or resumed.').waitFor();

      const tampered = await parseArchive(text);
      const genes = tampered.rows.find((row) => row.data.representative?.genes).data.representative.genes;
      [genes[0], genes[1]] = [genes[1], genes[0]];
      await panel.locator('input[type=file]').setInputFiles({
        name: 'unsorted.jsonl', mimeType: 'application/x-ndjson', buffer: Buffer.from(encodeArchive(tampered)),
      });
      await page.waitForFunction(() => /representative/.test(
        document.querySelector('.history-panel [role=status]')?.textContent ?? ''));
      for (const width of [390, 320]) {
        await page.setViewportSize({ width, height: 844 });
        assert.equal(await panel.evaluate((node) => node.scrollWidth > node.clientWidth), false);
      }
    });
  });

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
        assert.equal(await panel.getByRole('checkbox', { name: 'record the next new / reseeded run' }).isChecked(), false);
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

        await panel.getByRole('button', { name: /^delete archive / }).first().click();
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
      const exported = await download(page, panel.getByRole('button', { name: /^export for archive / }));
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

test('a real retune footer survives reseed while its IndexedDB append is delayed',
  { timeout: 60_000 }, async () => {
    await withPage('development', async (page) => {
      const result = await page.evaluate(async (params) => {
        const { createSim } = await import('/src/sim/client.js');
        const { createHistorySession } = await import('/src/history/controller.js');
        const { openHistoryStore } = await import('/src/history/store.js');
        const store = await openHistoryStore();
        let firstCommit;
        let closingStarted;
        let release;
        const first = new Promise((resolve) => { firstCommit = resolve; });
        const closing = new Promise((resolve) => { closingStarted = resolve; });
        const gate = new Promise((resolve) => { release = resolve; });
        let appends = 0;
        const delayedStore = {
          ...store,
          async append(...args) {
            const count = ++appends;
            if (count === 2) { closingStarted(); await gate; }
            const result = await store.append(...args);
            if (count === 1) firstCommit();
            return result;
          },
        };
        const sim = createSim({
          seed: '42', founders: 32, params: JSON.stringify(params), historyRunId: crypto.randomUUID(),
        });
        const session = createHistorySession({
          sim, getStore: async () => delayedStore, onChange() {}, onSaved: async () => {},
        });
        try {
          const { run } = await new Promise((resolve) => sim.on('ready', resolve));
          await first;
          const normalized = JSON.parse(run.params);
          sim.setParams(JSON.stringify({
            ...normalized, metabolism: { ...normalized.metabolism, base: 0.1 },
          }));
          await closing;
          let replaced = false;
          let error = null;
          const reseed = (async () => {
            try {
              await session.waitForBoundary();
              if (session.active) await session.boundary('reseeded');
            } catch (failure) {
              error = String(failure);
            }
            session.detach();
            sim.destroy();
            replaced = true;
          })();
          await new Promise((resolve) => setTimeout(resolve, 30));
          const replacedBeforeCommit = replaced;
          release();
          await reseed;
          const archive = await store.get(session.id);
          return {
            replacedBeforeCommit, replaced, error, status: archive.status,
            captureEnd: archive.completion?.data.capture_end,
            hash: archive.completion?.data.cohorts[0].final_state_hash,
            originalBase: archive.header.data.params.metabolism.base,
          };
        } finally {
          release();
          session.detach();
          sim.destroy();
          if (session.id) await store.delete(session.id);
          store.close();
        }
      }, params);
      assert.equal(result.replacedBeforeCommit, false);
      assert.equal(result.replaced, true);
      assert.equal(result.error, null);
      assert.equal(result.status, 'closed');
      assert.equal(result.captureEnd, 'params_changed');
      assert.equal(result.originalBase, 0.05);
      assert.match(result.hash, /^[0-9a-f]{16}$/);
    });
  });
