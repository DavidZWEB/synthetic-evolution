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

1. Start the dev server in the background and wait for Vite to print its ready
   URL:

   ```bash
   PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:$PATH" \
     npm run dev --prefix web
   ```

   It serves on `http://localhost:5173` and rebuilds the WASM bindings first.
   The explicit `PATH` covers the documented macOS/Homebrew rustup trap. If
   `vite` is missing, run `npm ci --prefix web`; if the Rust tools are genuinely
   absent rather than merely off `PATH`, run `./scripts/setup.sh --skip-tests`.
2. Open the page at a desktop viewport (1200×1280 is a useful baseline). Wait
   until transport is no longer `…`, then take both:
   - an accessibility snapshot, for exact labels, values, disabled states, and
     element bounds;
   - a full-page screenshot, for the canvas, overlays, clipping, and visual
     hierarchy.
3. Read console warnings/errors and inspect network requests. Confirm the WASM
   module returned 200 and report even harmless failures separately (for
   example, a missing favicon must not be described as an application or WebGL
   failure).
4. Exercise the behavior under review. For a broad smoke check, cover:
   - **Step:** tick moves from 0 to 1; energy and chart values update.
   - **Play/pause:** tick advances while playing, the step button disables, and
     tick remains fixed after pausing.
   - **Speed:** select a second speed and confirm the active state or faster tick
     advance, then return to 1×. Keep runs short: the Phase 1 economy can reach
     extinction quickly, which is not by itself a UI failure.
   - **Reseed:** change seed and founder count; confirm tick resets, population
     matches, the canvas changes, and the URL fragment reflects the active run.
   - **Camera:** wheel over the canvas; confirm the zoom readout and rendered
     scale change, then use reset view and confirm 1.0×.
   - **Inspector:** while paused on a dense founder run, click an agent, step
     once, and confirm identity/energy/age plus non-zero live activations update.
     Open the genome disclosure and verify its contents render and scroll.
   - **Sharing:** click copy link and confirm the button changes to `copied`.

   The changed feature takes priority over this checklist. Do not substitute a
   screenshot for exercising a behavioral control.
5. Check at least one narrow viewport, normally 390×844. A full-page screenshot
   may conceal horizontal overflow by expanding to the document's scroll width,
   so also inspect element bounds or compare
   `document.documentElement.scrollWidth` with `window.innerWidth`. Report
   clipping, overlap, unreachable controls, and horizontal scrolling even when
   desktop behavior is correct.
6. Stop the dev server, close the browser page, and delete temporary screenshots
   before finishing unless the user asked to keep them. Confirm the worktree was
   not dirtied by validation artifacts.

## Reporting

Lead with a pass/fail assessment and separate desktop behavior from responsive
findings. State what was on screen, what you clicked, the before/after values,
and any console or network output. If something looks wrong, include the
viewport, screenshot description, measured overflow or bounds, and exact error.
Distinguish a rendering/control defect from an expected simulation outcome such
as population collapse. This evidence is what a human uses for the repo's
"judgment" bar for UI and behavior changes.
