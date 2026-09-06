---
name: playwright-visual-check
description: Visually verify the Svelte + WebGL2 client using the Playwright MCP browser tools — take screenshots, read console/network errors, and inspect rendered DOM/canvas state. Use after any change under web/, and whenever fixing or validating a UI bug, to confirm the client actually renders and behaves as intended rather than relying on TypeScript checks alone.
---

# Playwright visual check

`npm run check --prefix web` and `npm test --prefix web` catch type errors and
logic bugs, but neither one renders the page. WebGL2 and Svelte runtime
issues (a blank canvas, a control that doesn't wire up, a console error only
thrown in the browser) only show up by actually loading the client. That's
what this skill is for.

## When to use this

- Any change under `web/` (components, stores, wasm bindings, WebGL renderer code).
- Fixing or validating a reported UI bug — confirm the bug reproduces, then
  confirm the fix visually, don't just trust the diff.
- Before handing off a PR touching `web/`, as a last sanity check.

Don't use it for changes confined to `sim-core`, `shells/native`, or docs.

## How to run it

1. Follow `docs/development.md` for the current setup, including its opt-in
   browser installation, then start the development script defined in
   `web/package.json` in the background. Read the URL Vite prints rather than
   assuming a host or port.
2. Open that URL at a representative desktop viewport. Wait until transport is
   no longer `…`, then take both:
   - an accessibility snapshot, for exact labels, values, disabled states, and
     element bounds;
   - a full-page screenshot, for the canvas, overlays, clipping, and visual
     hierarchy.
3. Read console warnings/errors and inspect network requests. Confirm the WASM
   module returned 200 and report even harmless failures separately (for
   example, a missing favicon must not be described as an application or WebGL
   failure).
4. Exercise the behavior under review. For a broad smoke check, cover:
   - **Step:** the tick advances by the requested single step; dependent status
     and chart values update.
   - **Play/pause:** tick advances while playing, the step button disables, and
     tick remains fixed after pausing.
   - **Speed:** select a second speed and confirm the active state or faster tick
     advance, then return to 1×. Keep runs short: the Phase 1 economy can reach
     extinction quickly, which is not by itself a UI failure.
   - **Reseed:** change seed and founder count; confirm tick resets, population
     matches, the canvas changes, and the URL fragment reflects the active run.
   - **Camera:** wheel over the canvas; confirm the zoom readout and rendered
     scale change, then use reset view and confirm the fitted whole-world view
     returns.
   - **Inspector:** while paused on a dense founder run, click an agent, step
     once, and confirm identity/energy/age plus live activations update.
     Open the genome disclosure and verify its contents render and scroll.
   - **Sharing:** click copy link and confirm visible success feedback.

   The changed feature takes priority over this checklist. Do not substitute a
   screenshot for exercising a behavioral control.
5. The normal development server sends the isolation headers that select the
   shared-buffer transport. Stop it, start the package script dedicated to the
   transferable fallback, and repeat the core load, step, and agent-selection
   checks. Confirm the transport readout identifies the path being exercised.
   If the change is unrelated to transport, a short fallback smoke check is
   enough; transport, snapshot, renderer, or picking changes need the full
   relevant interaction on both paths.
6. Check at least one representative narrow viewport. A full-page screenshot may
   conceal horizontal overflow by expanding to the document's scroll width, so
   also inspect element bounds or compare
   `document.documentElement.scrollWidth` with `window.innerWidth`. Report
   clipping, overlap, unreachable controls, and horizontal scrolling even when
   desktop behavior is correct.
7. A visual check supplements rather than replaces automated coverage. When it
   exposes a reproducible bug, add a regression test at the lowest useful layer
   before the branch lands; use browser-level coverage when the failure depends
   on real DOM, worker, or WebGL behavior.
8. Stop the dev server, close the browser page, and delete temporary screenshots
   before finishing unless the user asked to keep them. Confirm the worktree was
   not dirtied by validation artifacts.

## Reporting

Lead with a pass/fail assessment and separate desktop behavior from responsive
findings. State what was on screen, what you clicked, the before/after values,
which transport paths were exercised, and any console or network output. If
something looks wrong, include the viewport, screenshot description, measured
overflow or bounds, and exact error. Distinguish a rendering/control defect from
an expected simulation outcome such as population collapse. This evidence is
what a human uses for the repo's "judgment" bar for UI and behavior changes.
