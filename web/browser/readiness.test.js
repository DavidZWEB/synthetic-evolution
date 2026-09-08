/** Real worker, renderer, lifecycle, and responsive-layout regressions. */

import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';

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

async function withClient(mode, run, { beforeLoad, ready = true } = {}) {
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
    const url = new URL(server.resolvedUrls.local[0]);
    url.hash = new URLSearchParams({
      seed: '42',
      founders: String(founders),
      params: JSON.stringify({
        world: { max_agents: 128 },
        plants: { max_plants: 64 },
        chemo: { cells: [8, 8, 1] },
      }),
    }).toString();
    await page.goto(url.href);
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

for (const mode of ['development', 'transferable']) {
  test(`scalar control explains heredity and preserves the shared mode ID (${mode})`, async () => {
    await withClient(mode, async (page) => {
      const heredity = page.getByRole('combobox', { name: 'heredity', exact: true });
      const explanation = /inherits topology, which may evolve when structural mutation is enabled; neural scalars are redrawn at birth/;
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
            controls: [...document.querySelectorAll('button,input,select')].map((el) => ({
              name: el.textContent || el.getAttribute('type'),
              ...bounds(el),
            })),
          };
        });
        assert.ok(layout.scrollWidth <= layout.width, `horizontal overflow: ${JSON.stringify(layout)}`);
        assert.equal(layout.inspectorOverflows, false, 'inspector contents overflow horizontally');
        for (const element of [layout.inspector, ...layout.controls]) {
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
