---
name: playwright-visual-check
description: Visually verify the Svelte + Three.js web client using the Playwright MCP browser tools — take screenshots, read console/network errors, and inspect rendered DOM/canvas state. Use after any change under web/, and whenever fixing or validating a UI bug, to confirm the client actually renders and behaves as intended rather than relying on TypeScript checks alone.
license: MIT
---

# Playwright visual check

`npm run check --prefix web` and `npm test --prefix web` catch type errors and
logic bugs, but neither one renders the page. Three.js/WebGL and Svelte runtime
issues (a blank canvas, a control that doesn't wire up, a console error only
thrown in the browser) only show up by actually loading the client. That's
what this skill is for.

## When to use this

- Any change under `web/` (components, stores, wasm bindings, Three.js scene code).
- Fixing or validating a reported UI bug — confirm the bug reproduces, then
  confirm the fix visually, don't just trust the diff.
- Before handing off a PR touching `web/`, as a last sanity check.

Don't use it for changes confined to `sim-core`, `shells/native`, or docs.

## How to run it

1. Start the dev server in the background and wait for it to be ready:
   `npm run dev --prefix web` (serves on `http://localhost:5173`; rebuilds wasm
   bindings first, so give it time on the first run).
2. Use the Playwright MCP tools to open `http://localhost:5173`, take a
   screenshot, and read the browser console for errors/warnings. Interact with
   whatever control is relevant to the change (drag, click, param slider,
   etc.) and screenshot again to confirm the resulting state.
3. Compare against what the change is supposed to do. A clean screenshot with
   no console errors is not sufficient on its own if the feature is behavioral
   (e.g. a slider should change sim speed) — actually exercise it.
4. Stop the dev server when done.

## Reporting

State what you saw, not just "looks fine": what was on screen, what you
clicked, what changed, and any console output. If something looks wrong,
include the screenshot description and the exact console error — this is
what a human will use to judge the change, per this repo's "judgment" bar
for UI/behavior changes.
