/** Real worker, renderer, lifecycle, and responsive-layout regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';

import { readRunUrl, writeRunUrl } from '../src/sim/seed-url.js';

const root = fileURLToPath(new URL('../', import.meta.url));
const founders = 128;

async function waitForTransport(page, mode) {
  const transport = mode === 'transferable' ? 'transferable' : 'shared';
  await page.waitForFunction(
    (expected) =>
      [...document.querySelectorAll('header dt')].find((el) => el.textContent === 'transport')
        ?.nextElementSibling?.textContent === expected,
    transport,
    { timeout: 10000 },
  );
  assert.equal(await page.evaluate(() => crossOriginIsolated), mode !== 'transferable');
}

async function withClient(mode, run, { beforeLoad, ready = true, params = {}, brainInheritance } = {}) {
  const server = await createServer({
    root,
    mode,
    logLevel: 'silent',
    server: { host: '127.0.0.1', strictPort: false },
  });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await beforeLoad?.(page);
    const url = writeRunUrl(server.resolvedUrls.local[0], {
      seed: '42',
      founders,
      brainInheritance,
      params: JSON.stringify({
        world: { max_agents: 128 },
        plants: { max_plants: 64 },
        chemo: { cells: [8, 8, 1] },
        ...params,
      }),
    });
    await page.goto(url);
    if (ready) await waitForTransport(page, mode);
    await run(page);
    assert.deepEqual(errors, [], 'the browser raised an application error');
  } finally {
    try {
      await browser?.close();
    } finally {
      await server.close();
    }
  }
}

async function firstFounderPoint(page) {
  const canvas = await page.locator('canvas').first().boundingBox();
  assert.ok(canvas && canvas.width > 0 && canvas.height > 0, 'world canvas is not visible');
  const scale = Math.min(canvas.width, canvas.height) / 1000;
  const offset = 400 * Math.sqrt(0.5 / founders);
  return {
    x: canvas.x + canvas.width / 2 + offset * scale,
    y: canvas.y + canvas.height / 2,
    worldWidth: 1000 * scale,
  };
}

async function selectFirstFounder(page) {
  const point = await firstFounderPoint(page);
  await page.mouse.click(point.x, point.y);
  const inspector = page.getByRole('complementary', { name: 'Inspector for agent 0', exact: true });
  await inspector.waitFor();
  return inspector;
}

async function inspectorFields(inspector) {
  return inspector.evaluate((el) => Object.fromEntries(
    [...el.querySelectorAll('dt')].map((dt) => [dt.textContent, dt.nextElementSibling.textContent]),
  ));
}

for (const mode of ['development', 'transferable']) {
  test(`stable birth inspection survives stepping and resets with its world (${mode})`, async () => {
    await withClient(mode, async (page) => {
      let inspector = await selectFirstFounder(page);
      await page.waitForFunction(() => [...document.querySelectorAll('aside dt')]
        .find((dt) => dt.textContent === 'birth ID')?.nextElementSibling?.textContent === '#0 (this world)');
      const before = await inspectorFields(inspector);
      assert.match(await inspector.locator('header strong').innerText(), /^pool slot #0$/);
      assert.equal(before['parent A birth ID'], '— (founder)');
      assert.equal(before['parent B birth ID'], '— (asexual)');
      assert.equal(before['parent A slot'], '— (founder)');
      assert.equal(before['parent B slot'], '— (asexual)');
      assert.ok(Object.hasOwn(before, 'species'));
      await inspector.locator('summary').click();
      await page.waitForFunction(() => (document.querySelector('pre')?.textContent.length ?? 0) > 0);
      const genome = await inspector.locator('pre').innerText();
      const activations = await inspector.locator('.neuron output').allTextContents();
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() =>
        document.querySelector('aside dt')?.nextElementSibling?.textContent === '1');
      const after = await inspectorFields(inspector);
      assert.equal(after['birth ID'], before['birth ID']);
      assert.equal(after['slot incarnation'], before['slot incarnation']);
      assert.equal(after.age, '1');
      assert.equal(await inspector.locator('pre').innerText(), genome);
      assert.notDeepEqual(await inspector.locator('.neuron output').allTextContents(), activations);

      await page.getByRole('textbox', { name: 'seed', exact: true }).fill('117');
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await page.waitForFunction(() => new URLSearchParams(location.hash.slice(1)).get('seed') === '117');
      await waitForTransport(page, mode);
      assert.equal(await page.getByRole('complementary').count(), 0, 'selection survived world reset');
      inspector = await selectFirstFounder(page);
      await page.waitForFunction(() => [...document.querySelectorAll('aside dt')]
        .find((dt) => dt.textContent === 'birth ID')?.nextElementSibling?.textContent === '#0 (this world)');
      const reset = await inspectorFields(inspector);
      assert.equal(reset.tick, '0');
      assert.equal(reset.age, '0');
      assert.equal(reset['slot incarnation'], '1');
      assert.equal(reset['parent A birth ID'], '— (founder)');
      assert.equal(await inspector.locator('details').evaluate((el) => el.open), false);
    });
  });

  test(`high-u64 and unavailable parent identities render without slot ancestry or overflow (${mode})`, async () => {
    await withClient(mode, async (page) => {
      const inspector = await selectFirstFounder(page);
      await page.waitForFunction(() => [...document.querySelectorAll('aside dt')]
        .find((dt) => dt.textContent === 'birth ID')?.nextElementSibling?.textContent
          === '#18446744073709551614 (this world)');
      const fields = await inspectorFields(inspector);
      assert.equal(fields['parent A birth ID'], '#9007199254740993 (this world)');
      assert.equal(fields['parent B birth ID'], '#9007199254740994 (this world)');
      assert.equal(fields['parent A slot'], 'slot #0 at birth (may be reused)');
      assert.equal(fields['parent B slot'], 'slot #0 at birth (may be reused)');
      for (const viewport of [
        { width: 1440, height: 1000 }, { width: 390, height: 844 }, { width: 320, height: 640 },
      ]) {
        await page.setViewportSize(viewport);
        const layout = await inspector.evaluate((el) => ({
          overflow: el.scrollWidth > el.clientWidth,
          documentOverflow: document.documentElement.scrollWidth > innerWidth,
          fields: [...el.querySelectorAll('.identity')].map((row) => {
            const label = row.querySelector('dt').getBoundingClientRect();
            const value = row.querySelector('dd').getBoundingClientRect();
            return { labelRight: label.right, valueLeft: value.left, valueRight: value.right };
          }),
        }));
        assert.equal(layout.overflow, false, `inspector overflow at ${viewport.width}px`);
        assert.equal(layout.documentOverflow, false, `document overflow at ${viewport.width}px`);
        for (const field of layout.fields) {
          assert.ok(field.labelRight <= field.valueLeft && field.valueRight <= viewport.width,
            `identity fields overlap at ${viewport.width}px: ${JSON.stringify(field)}`);
        }
      }
      await page.evaluate(() => {
        globalThis.inspectionIdentityFixture = {
          birth_id: null, parent_birth_a: null, parent_birth_b: null, parent_a: 0, parent_b: 0,
        };
      });
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() => [...document.querySelectorAll('aside dt')]
        .find((dt) => dt.textContent === 'birth ID')?.nextElementSibling?.textContent === 'unavailable');
      const unavailable = await inspectorFields(inspector);
      assert.equal(unavailable['parent A birth ID'], 'unavailable');
      assert.equal(unavailable['parent B birth ID'], 'unavailable');
      assert.doesNotMatch(await inspector.innerText(), /founder|asexual/);
    }, {
      beforeLoad: (page) => page.addInitScript(() => {
        globalThis.inspectionIdentityFixture = {
          birth_id: '18446744073709551614',
          parent_birth_a: '9007199254740993',
          parent_birth_b: '9007199254740994',
          parent_a: 0,
          parent_b: 0,
        };
        const RealWorker = globalThis.Worker;
        globalThis.Worker = class extends RealWorker {
          constructor(...args) {
            super(...args);
            this.addEventListener('message', (event) => {
              if (event.data.kind === 'inspection' && event.data.agent) {
                event.data.agent = JSON.stringify({
                  ...JSON.parse(event.data.agent), ...globalThis.inspectionIdentityFixture,
                });
              }
            });
          }
        };
      }),
    });
  });

  test(`species status and inspection reflect real classification through reseed and stepping (${mode})`, async () => {
    for (const capacity of [0, 4]) {
      await withClient(mode, async (page) => {
        async function assertSpeciesStatus(expectedTick) {
          await page.waitForFunction(
            ({ capacity, founders, expectedTick }) => {
              const status = Object.fromEntries(
                [...document.querySelectorAll('header dt')]
                  .map((dt) => [dt.textContent, dt.nextElementSibling.textContent]),
              );
              return status.tick === expectedTick
                && status.agents === String(founders)
                && status.species === String(capacity)
                && status.unclassified === String(founders - capacity);
            },
            { capacity, founders, expectedTick },
          );
          assert.equal(await page.getByRole('alert').count(), 0);
        }
        async function assertInspectorSpecies() {
          const inspector = await selectFirstFounder(page);
          const expected = capacity === 0 ? 'unclassified' : '#0 (this world)';
          await page.waitForFunction(
            (expected) => [...document.querySelectorAll('aside dt')]
              .find((dt) => dt.textContent === 'species')?.nextElementSibling?.textContent === expected,
            expected,
          );
          assert.doesNotMatch(await inspector.innerText(), /placeholder|4294967295/);
        }
        await assertSpeciesStatus('0');
        await assertInspectorSpecies();
        const run = readRunUrl(page.url(), founders);
        assert.deepEqual(JSON.parse(run.params).species, { capacity, threshold: 1e-12 });
        await page.reload();
        await waitForTransport(page, mode);
        await assertSpeciesStatus('0');
        await assertInspectorSpecies();
        await page.getByRole('button', { name: 'step', exact: true }).click();
        await assertSpeciesStatus('1');
        await page.waitForFunction(() =>
          document.querySelector('aside dt')?.nextElementSibling?.textContent === '1');
        await page.getByRole('combobox', { name: 'heredity', exact: true })
          .selectOption('randomized_at_birth');
        await page.getByRole('button', { name: 'reseed', exact: true }).click();
        await page.waitForFunction(() =>
          new URLSearchParams(location.hash.slice(1)).get('inheritance') === 'randomized_at_birth');
        await waitForTransport(page, mode);
        await assertSpeciesStatus('0');
        await assertInspectorSpecies();
        await page.getByRole('button', { name: 'step', exact: true }).click();
        await assertSpeciesStatus('1');
        assert.deepEqual(JSON.parse(readRunUrl(page.url(), founders).params).species, {
          capacity, threshold: 1e-12,
        });
      }, {
        params: {
          species: { capacity, threshold: 1e-12 },
          brain: { connections_per_target: 1 },
        },
      });
    }
  });

  test(`scalar control explains heredity and preserves the shared mode ID (${mode})`, async () => {
    await withClient(mode, async (page) => {
      const heredity = page.getByRole('combobox', { name: 'heredity', exact: true });
      const explanation = /inherits topology and sensors, which may evolve when their mutation rates are enabled; neural scalars are redrawn at birth/;
      assert.match(await heredity.getAttribute('title'), explanation);
      assert.match(await heredity.getAttribute('aria-description'), explanation);
      assert.equal(
        await heredity.locator('option[value="randomized_at_birth"]').innerText(),
        'scalar control',
      );
      await heredity.selectOption({ label: 'scalar control' });
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await page.waitForFunction(() =>
        new URLSearchParams(location.hash.slice(1)).get('inheritance') === 'randomized_at_birth');
      await waitForTransport(page, mode);
      await page.reload();
      await waitForTransport(page, mode);
      assert.equal(await heredity.inputValue(), 'randomized_at_birth');
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() =>
        [...document.querySelectorAll('header dt')].find((el) => el.textContent === 'tick')
          ?.nextElementSibling?.textContent === '1');
    });
  });

  test(`sparse no-eye founders survive URL reload, heredity selection and inspection (${mode})`, async () => {
    const params = {
      sensing: { vision_rays: 0, chemo_sensors: 1, energy_sensors: 0 },
      brain: { hidden_neurons: 0, oscillators: 0, connections_per_target: 1 },
    };
    await withClient(mode, async (page) => {
      async function readGenome() {
        const inspector = await selectFirstFounder(page);
        await page.waitForFunction(() => document.querySelectorAll('aside .neuron').length === 7);
        assert.equal(await inspector.locator('.neuron').count(), 7);
        await inspector.locator('summary').click();
        await page.waitForFunction(() => (document.querySelector('pre')?.textContent.length ?? 0) > 0);
        const genome = JSON.parse(await inspector.locator('pre').innerText());
        const sensors = genome.filter((gene) => gene.Sensor).map((gene) => gene.Sensor);
        assert.equal(sensors.length, 1);
        assert.equal(sensors[0].modality, 'Chemo');
        assert.equal(genome.filter((gene) => gene.Neuron).length, 7);
        assert.equal(genome.filter((gene) => gene.Connection).length, 4);
        assert.equal(genome.filter((gene) => gene.Effector).length, 4);
        assert.equal(genome.filter((gene) => gene.Body).length, 4);
        assert.equal(genome.filter((gene) => gene.Meta).length, 3);
        return genome;
      }
      const original = await readGenome();
      const run = readRunUrl(page.url(), founders);
      const canonical = JSON.parse(run.params);
      assert.equal(canonical.sensing.vision_rays, 0);
      assert.equal(canonical.sensing.chemo_sensors, 1);
      assert.equal(canonical.sensing.energy_sensors, 0);
      assert.equal(canonical.brain.connections_per_target, 1);
      assert.equal(run.brainInheritance, 'randomized_at_birth');
      await page.reload();
      await waitForTransport(page, mode);
      assert.deepEqual(await readGenome(), original, 'shared URL did not reproduce the founder');
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(() =>
        document.querySelector('aside dt')?.nextElementSibling?.textContent === '1');
      assert.equal(await page.locator('aside .neuron').count(), 7);
      await page.getByRole('combobox', { name: 'heredity', exact: true }).selectOption('evolving');
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await page.waitForFunction(() => !new URLSearchParams(location.hash.slice(1)).has('inheritance'));
      await waitForTransport(page, mode);
      assert.deepEqual(await readGenome(), original, 'heredity changed the founding template');
    }, { params, brainInheritance: 'randomized_at_birth' });
  });

  test(`paused inspection catches a manual step (${mode})`, async () => {
    await withClient(mode, async (page) => {
      await selectFirstFounder(page);
      await page.getByRole('button', { name: 'step', exact: true }).click();
      await page.waitForFunction(
        () => document.querySelector('aside dt')?.nextElementSibling?.textContent === '1',
        undefined,
        { timeout: 5000 },
      );
      const fields = await page.locator('aside').evaluate((el) =>
        Object.fromEntries(
          [...el.querySelectorAll('dt')].map((dt) => [dt.textContent, dt.nextElementSibling.textContent]),
        ),
      );
      assert.equal(fields.age, '1');
      assert.equal(fields['slot incarnation'], '1');
    });
  });

  test(`blank margins do not select undisplayed toroidal copies (${mode})`, async () => {
    await withClient(mode, async (page) => {
      await page.setViewportSize({ width: 2000, height: 600 });
      const point = await firstFounderPoint(page);
      assert.ok(point.x - point.worldWidth > 0, 'the copy must fall inside a visible margin');
      await page.mouse.click(point.x - point.worldWidth, point.y);
      await page.evaluate(() => new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve))));
      assert.equal(await page.getByRole('complementary').count(), 0, 'picked an invisible copy');
      await selectFirstFounder(page);
    });
  });

  test(`reseed recovers after a fatal worker error (${mode})`, async () => {
    await withClient(mode, async (page) => {
      await page.getByRole('alert').waitFor();
      assert.match(await page.getByRole('alert').innerText(), /worker: forced bootstrap failure/);
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await waitForTransport(page, mode);
      assert.deepEqual(
        await page.evaluate(() => globalThis.workerLifecycle),
        { created: 2, terminated: 1 },
      );
      assert.equal(await page.getByRole('alert').count(), 0);
    }, {
      ready: false,
      beforeLoad: (page) => page.addInitScript(() => {
        const RealWorker = globalThis.Worker;
        const lifecycle = { created: 0, terminated: 0 };
        globalThis.workerLifecycle = lifecycle;
        globalThis.Worker = class {
          constructor(url, options) {
            lifecycle.created += 1;
            if (lifecycle.created > 1) return new RealWorker(url, options);
          }
          postMessage(message) {
            if (message.kind === 'create') {
              queueMicrotask(() => this.onerror?.({
                preventDefault() {},
                message: 'forced bootstrap failure',
              }));
            }
          }
          terminate() {
            lifecycle.terminated += 1;
          }
        };
      }),
    });
  });

  test(`controls and inspector stay inside narrow viewports (${mode})`, async () => {
    await withClient(mode, async (page) => {
      const inspector = await selectFirstFounder(page);
      await inspector.locator('summary').click();
      await page.waitForFunction(() => (document.querySelector('pre')?.textContent.length ?? 0) > 0);
      for (const viewport of [{ width: 390, height: 844 }, { width: 320, height: 640 }]) {
        await page.setViewportSize(viewport);
        const layout = await page.evaluate(() => {
          const bounds = (el) => {
            const rect = el.getBoundingClientRect();
            return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
          };
          const inspector = document.querySelector('aside');
          return {
            width: innerWidth,
            height: innerHeight,
            scrollWidth: document.documentElement.scrollWidth,
            inspector: bounds(inspector),
            inspectorOverflows: inspector.scrollWidth > inspector.clientWidth,
            status: [...document.querySelectorAll('header dl div')].map((el) => ({
              name: el.querySelector('dt').textContent,
              ...bounds(el),
            })),
            controls: [...document.querySelectorAll('button,input,select')].map((el) => ({
              name: el.textContent || el.getAttribute('type'),
              ...bounds(el),
            })),
          };
        });
        assert.ok(layout.scrollWidth <= layout.width, `horizontal overflow: ${JSON.stringify(layout)}`);
        assert.equal(layout.inspectorOverflows, false, 'inspector contents overflow horizontally');
        assert.ok(layout.status.some((field) => field.name === 'species'));
        assert.ok(layout.status.some((field) => field.name === 'unclassified'));
        for (const element of [layout.inspector, ...layout.controls, ...layout.status]) {
          assert.ok(
            element.left >= 0 && element.right <= layout.width
              && element.top >= 0 && element.bottom <= layout.height,
            `offscreen element at ${viewport.width}px: ${JSON.stringify(element)}`,
          );
        }
      }
      await inspector.getByRole('button', { name: 'Close inspector', exact: true }).click();
      await inspector.waitFor({ state: 'detached' });
      await page.getByRole('button', { name: '2\u00d7', exact: true }).click();
      await page.waitForFunction(() => [...document.querySelectorAll('button')]
        .some((button) => button.textContent.trim() === '2\u00d7' && button.classList.contains('active')));
      await page.getByRole('button', { name: '1\u00d7', exact: true }).click();
      await page.getByRole('textbox', { name: 'seed', exact: true }).fill('117');
      await page.getByRole('spinbutton', { name: 'founders', exact: true }).fill('64');
      await page.getByRole('button', { name: 'reseed', exact: true }).click();
      await page.waitForFunction(() => {
        const params = new URLSearchParams(location.hash.slice(1));
        return params.get('seed') === '117' && params.get('founders') === '64';
      });
    });
  });
}
