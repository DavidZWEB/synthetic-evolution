/** Real IndexedDB transaction, quota, reload and portable archive regressions. */
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import { writeRunUrl } from '../src/sim/seed-url.js';

let server;
let browser;
let url;

before(async () => {
  server = await createServer({
    root: fileURLToPath(new URL('../', import.meta.url)), logLevel: 'silent',
    server: { host: '127.0.0.1', strictPort: false },
  });
  await server.listen();
  browser = await chromium.launch({ headless: true });
  url = writeRunUrl(server.resolvedUrls.local[0], {
    seed: '42', founders: 8,
    params: JSON.stringify({
      world: { max_agents: 32 }, plants: { max_plants: 16 }, chemo: { cells: [8, 8, 1] },
    }),
  });
});

after(async () => {
  await browser?.close();
  await server?.close();
});

async function load(page) {
  await page.goto(url);
  await page.waitForFunction(() => [...document.querySelectorAll('header dt')]
    .find((node) => node.textContent === 'transport')?.nextElementSibling?.textContent === 'shared');
  await page.evaluate(async () => {
    globalThis.historyTest = {
      ...await import('/src/history/store.js'),
      ...await import('/src/history/archive.js'),
      ...await import('/src/history/fixtures.js'),
    };
  });
}

async function withPage(run) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  try {
    await load(page);
    await run(page, context);
    assert.deepEqual(errors, []);
  } finally {
    await context.close();
  }
}

test('IndexedDB imports native paired v1 and v2 without merging cohort namespaces or run IDs', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const wasm = await import('/src/wasm/wasm.js');
      await wasm.default();
      const nativeShape = h.fixtureArchive(1);
      nativeShape.header.data.params = JSON.parse(wasm.validate_params(JSON.stringify(nativeShape.header.data.params)));
      const v1 = await h.parseArchive(h.encodeArchive(nativeShape), (params) =>
        wasm.validate_params(JSON.stringify(params)));
      const v1Id = await store.importArchive(v1);
      const v2 = h.fixtureArchive();
      const a = await store.importArchive(v2);
      const b = await store.importArchive(v2);
      const saved = await store.get(v1Id);
      const exported = await h.parseArchive(await store.exportArchive(v1Id));
      const records = await store.list();
      const mutations = await Promise.allSettled([
        store.append(v1Id, [], { tick: '1' }),
        store.append(a, [], { tick: '1' }),
      ]);
      store.close();
      return {
        v1Id, a, b, saved, exported, records,
        codes: mutations.map((result) => result.reason?.code),
      };
    });
    assert.notEqual(result.a, result.b);
    assert.notEqual(result.a, 'fixture-run');
    assert.deepEqual(result.saved.cohorts, ['evolving', 'random_control']);
    assert.equal(result.saved.eventCount, '2');
    assert.equal(result.saved.rows[0].data.sequence, '0');
    assert.equal(result.saved.rows[1].data.sequence, '0');
    assert.equal(result.exported.header.data.schema_version, 1);
    assert.equal(result.exported.completion.data.cohorts.length, 2);
    assert.deepEqual(result.codes, ['read_only', 'read_only']);
    assert.equal(result.records.length, 3);
    assert.ok(result.records.every((record) => record.status === 'closed'));
  });
});

test('live per-run byte limits reserve export space and abort the whole rejected batch', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const header = h.fixtureHeader();
      const initial = h.byteLength(h.encodeLine(header)) + h.footerAllowance(header);
      const max = initial + h.byteLength(h.encodeLine(h.origin()));
      const store = await h.openHistoryStore({ limits: { perRunBytes: max, totalBytes: max } });
      const id = await store.create(header);
      const first = await store.append(id, [h.origin()], { tick: '0' });
      let error;
      try { await store.append(id, [h.gap('1', '1')], { tick: '5' }); }
      catch (caught) { error = caught.code; }
      const retained = await store.get(id);
      await store.markIncomplete(id, 'storage_limit');
      const text = await store.exportArchive(id);
      const archive = await h.parseArchive(text);
      await store.delete(id);
      const imported = await store.importArchive(archive);
      const importedMeta = await store.get(imported);
      store.close();
      return { max, first, error, retained, archive, textBytes: h.byteLength(text), importedMeta };
    });
    assert.equal(result.first.bytes, result.max);
    assert.equal(result.error, 'storage_limit');
    assert.equal(result.retained.rows.length, 1);
    assert.equal(result.retained.tick, '0');
    assert.equal(result.retained.bytes, result.max);
    assert.equal(result.archive.completion.data.ticks, '0');
    assert.equal(result.archive.completion.data.capture_end, 'storage_limit');
    assert.equal(result.archive.completion.data.cohorts[0].history_complete, false);
    assert.ok(result.textBytes <= result.max);
    assert.equal(result.importedMeta.bytes, result.textBytes);
    assert.equal(result.importedMeta.status, 'closed');
  });
});

test('exact serialized import limits reject one extra byte without creating or evicting runs', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const archive = h.fixtureArchive();
      const bytes = h.byteLength(h.encodeArchive(archive));
      const tooSmall = await h.openHistoryStore({ limits: { perRunBytes: bytes - 1 } });
      let code;
      try { await tooSmall.importArchive(archive); } catch (error) { code = error.code; }
      const rejected = await tooSmall.list();
      tooSmall.close();
      const exact = await h.openHistoryStore({ limits: { perRunBytes: bytes, totalBytes: bytes } });
      const id = await exact.importArchive(archive);
      let second;
      try { await exact.importArchive(archive); } catch (error) { second = error.code; }
      const saved = await exact.list();
      exact.close();
      return { code, rejected, id, second, saved, bytes };
    });
    assert.equal(result.code, 'storage_limit');
    assert.equal(result.rejected.length, 0);
    assert.equal(result.second, 'storage_limit');
    assert.equal(result.saved.length, 1);
    assert.equal(result.saved[0].id, result.id);
    assert.equal(result.saved[0].bytes, result.bytes);
  });
});

test('header-only creation reserves the footer before it consumes a run slot', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const header = h.fixtureHeader();
      const bytes = h.byteLength(h.encodeLine(header)) + h.footerAllowance(header);
      const store = await h.openHistoryStore({ limits: { perRunBytes: bytes - 1 } });
      let code;
      try { await store.create(header); } catch (error) { code = error.code; }
      const list = await store.list();
      store.close();
      return { code, list };
    });
    assert.equal(result.code, 'storage_limit');
    assert.deepEqual(result.list, []);
  });
});

for (const quota of ['totalBytes', 'maxRuns']) {
  test(`two tabs cannot race past the shared ${quota} quota`, async () => {
    await withPage(async (page, context) => {
      const other = await context.newPage();
      await load(other);
      const config = await page.evaluate((quota) => {
        const h = globalThis.historyTest;
        const header = h.fixtureHeader();
        const bytes = h.byteLength(h.encodeLine(header)) + h.footerAllowance(header);
        return { limits: quota === 'totalBytes' ? { totalBytes: bytes * 2 - 1 } : { maxRuns: 1 } };
      }, quota);
      await Promise.all([page, other].map((tab) => tab.evaluate(async (config) => {
        globalThis.connection = await globalThis.historyTest.openHistoryStore(config);
      }, config)));
      const attempts = await Promise.all([page, other].map((tab) => tab.evaluate(async () => {
        try { return { id: await globalThis.connection.create(globalThis.historyTest.fixtureHeader()) }; }
        catch (error) { return { code: error.code }; }
      })));
      assert.equal(attempts.filter((result) => result.id).length, 1);
      assert.equal(attempts.filter((result) => result.code === 'storage_limit').length, 1);
      assert.equal(await other.evaluate(async () => (await globalThis.connection.list()).length), 1);
    });
  });
}

test('concurrent connections validate append against the latest committed sequence', async () => {
  await withPage(async (page, context) => {
    const other = await context.newPage();
    await load(other);
    const id = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      globalThis.connection = await h.openHistoryStore();
      return globalThis.connection.create(h.fixtureHeader());
    });
    await other.evaluate(async () => {
      globalThis.connection = await globalThis.historyTest.openHistoryStore();
    });
    const results = await Promise.all([page, other].map((tab) => tab.evaluate(async (id) => {
      try {
        await globalThis.connection.append(id, [globalThis.historyTest.origin()], { tick: '1' });
        return 'committed';
      } catch (error) { return error.message; }
    }, id)));
    assert.equal(results.filter((value) => value === 'committed').length, 1);
    assert.match(results.find((value) => value !== 'committed'), /contiguous/);
    const retained = await page.evaluate((id) => globalThis.connection.get(id), id);
    assert.equal(retained.rows.length, 1);
    assert.equal(retained.eventCount, '1');
  });
});

test('deleted and finalized archives cannot be recreated by stale append/finalize calls', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const header = h.fixtureHeader();
      const deleted = await store.create(header);
      await store.delete(deleted);
      const footer = h.completionFor(header, [], { tick: '0', captureEnd: 'unfinalized' });
      const afterDelete = await Promise.allSettled([
        store.append(deleted, [h.origin()], { tick: '0' }), store.finalize(deleted, footer),
      ]);
      const final = await store.create(header);
      await store.append(final, [h.origin()], { tick: '1' });
      const completion = h.completionFor(header, [h.origin()], {
        tick: '1', captureEnd: 'stopped', stateHash: '0000000000000000',
      });
      const closed = await store.finalize(final, completion);
      const afterFinal = await Promise.allSettled([
        store.append(final, [h.gap('1', '1')], { tick: '2' }), store.finalize(final, completion),
      ]);
      const list = await store.list();
      store.close();
      return {
        afterDelete: afterDelete.map((result) => result.reason?.code),
        afterFinal: afterFinal.map((result) => result.reason?.code), closed, list,
      };
    });
    assert.deepEqual(result.afterDelete, ['not_found', 'not_found']);
    assert.deepEqual(result.afterFinal, ['read_only', 'read_only']);
    assert.equal(result.closed.captureEnd, 'stopped');
    assert.equal(result.list.length, 1);
    assert.equal(result.list[0].status, 'closed');
  });
});

test('browser quota errors roll back queued archive writes and accounting together', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const header = h.fixtureHeader();
      const id = await store.create(header);
      await store.append(id, [h.origin()], { tick: '1' });
      const before = await store.get(id);
      const put = IDBObjectStore.prototype.put;
      let error;
      IDBObjectStore.prototype.put = function (...args) {
        if (this.name === 'accounting') throw new DOMException('Injected quota exhaustion', 'QuotaExceededError');
        return put.apply(this, args);
      };
      try { await store.append(id, [h.gap('1', '2')], { tick: '9' }); }
      catch (caught) { error = { code: caught.code, cause: caught.cause?.name }; }
      finally { IDBObjectStore.prototype.put = put; }
      const retained = await store.get(id);
      await store.markIncomplete(id, 'storage_error');
      const exported = await h.parseArchive(await store.exportArchive(id));
      await store.delete(id);
      const exact = await h.openHistoryStore({ limits: { totalBytes: before.bytes } });
      const next = await exact.create(header);
      await exact.append(next, [h.origin()], { tick: '1' });
      const available = (await exact.list()).length;
      exact.close();
      store.close();
      return { before, retained, error, exported, available };
    });
    assert.deepEqual(result.error, { code: 'storage_error', cause: 'QuotaExceededError' });
    assert.deepEqual(result.retained, result.before);
    assert.equal(result.exported.completion.data.ticks, '1');
    assert.equal(result.exported.completion.data.capture_end, 'storage_error');
    assert.equal(result.available, 1, 'rolled back bytes must not poison shared accounting');
  });
});

test('snapshot encoding leaves the live run open and fixes the prefix before subsequent appends', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const header = h.fixtureHeader();
      const id = await store.create(header);
      await store.append(id, [h.origin()], { tick: '1' });
      const snapshot = await store.get(id);
      snapshot.completion = h.completionFor(header, snapshot.rows, {
        tick: '1', captureEnd: 'snapshot', stateHash: '0000000000000000',
      });
      const text = h.encodeArchive(snapshot);
      await store.append(id, [h.gap('1', '1')], { tick: '2' });
      const live = await store.get(id);
      const exported = await h.parseArchive(text);
      store.close();
      return { live, exported };
    });
    assert.equal(result.live.status, 'open');
    assert.equal(result.live.captureEnd, null);
    assert.equal(result.live.completion, null);
    assert.equal(result.live.rows.length, 2);
    assert.equal(result.live.tick, '2');
    assert.equal(result.exported.rows.length, 1);
    assert.equal(result.exported.completion.data.ticks, '1');
    assert.equal(result.exported.completion.data.capture_end, 'snapshot');
  });
});

test('reload lists possibly-active prefixes and exports only the exact committed boundary', async () => {
  await withPage(async (page) => {
    const id = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const id = await store.create(h.fixtureHeader());
      await store.append(id, [h.origin()], { tick: '9007199254740993' });
      return id;
    });
    await load(page);
    const result = await page.evaluate(async (id) => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const list = await store.list();
      const exported = await h.parseArchive(await store.exportArchive(id));
      store.close();
      return { list, exported };
    }, id);
    assert.equal(result.list.length, 1);
    assert.equal(result.list[0].id, id);
    assert.equal(result.list[0].status, 'open');
    assert.equal(result.list[0].captureEnd, null);
    assert.equal(result.exported.completion.data.capture_end, 'unfinalized');
    assert.equal(result.exported.completion.data.ticks, '9007199254740993');
    assert.equal(result.exported.completion.data.cohorts[0].history_complete, false);
  });
});

test('invalid appends and footer boundaries preserve the previously committed prefix', async () => {
  await withPage(async (page) => {
    const result = await page.evaluate(async () => {
      const h = globalThis.historyTest;
      const store = await h.openHistoryStore();
      const header = h.fixtureHeader();
      const id = await store.create(header);
      await store.append(id, [h.origin()], { tick: '1' });
      const footer = h.completionFor(header, [h.origin()], {
        tick: '2', captureEnd: 'snapshot', stateHash: '0000000000000000',
      });
      const rejected = await Promise.allSettled([
        store.append(id, [h.origin('1', 0, '1')], { tick: '2' }),
        store.append(id, [], { tick: '0' }),
        store.finalize(id, footer),
      ]);
      const retained = await store.get(id);
      store.close();
      return { rejected: rejected.map((result) => result.status), retained };
    });
    assert.deepEqual(result.rejected, ['rejected', 'rejected', 'rejected']);
    assert.equal(result.retained.rows.length, 1);
    assert.equal(result.retained.tick, '1');
    assert.equal(result.retained.status, 'open');
  });
});
