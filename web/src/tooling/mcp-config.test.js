/** Keeps the repository MCP launcher on the exact version pinned in package.json. */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function readJson(relativeUrl) {
  return JSON.parse(await readFile(new URL(relativeUrl, import.meta.url), 'utf8'));
}

test('Playwright MCP config uses the package version recorded by npm', async () => {
  const [mcpConfig, packageJson] = await Promise.all([
    readJson('../../../.github/mcp.json'),
    readJson('../../package.json'),
  ]);
  const args = mcpConfig.mcpServers.playwright.args;
  const configuredPackage = args.find((argument) =>
    argument.startsWith('@playwright/mcp@'),
  );

  assert.equal(
    configuredPackage,
    `@playwright/mcp@${packageJson.devDependencies['@playwright/mcp']}`,
  );
  assert.deepEqual(
    args.slice(args.indexOf('--browser'), args.indexOf('--browser') + 2),
    ['--browser', 'chromium'],
  );
});
