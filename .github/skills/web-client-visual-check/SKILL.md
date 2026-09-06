---
name: web-client-visual-check
description: Load the Svelte + WebGL2 web client in a browser and verify it renders and responds — screenshots, console and network errors, and the behaviour of the controls under review. Use after any change under web/, and when reproducing or confirming a UI bug, because the type and unit checks never render the page.
---

# Web client visual check

`npm run check --prefix web` and `npm test --prefix web` catch type errors and
browser-independent logic bugs. Neither one draws a frame. WebGL2 and Svelte
runtime failures — a blank canvas, a control that never wires up, an exception
thrown only in the browser — are invisible to both. Three from M9 and M10, each
caught only by loading the page: a vertex attribute declared `normalized`
divided the alive flag by 255 and rendered an empty world with no error at all;
a backtick inside a GLSL comment terminated the shader template and blanked the
page; a throw inside the worker's timer loop froze the clock while the renderer
kept drawing the last frame.

Loading the page is the only thing that catches those.

## When to use this

- Any change under `web/` — components, stores, renderer, worker, transport,
  or regenerated WASM bindings.
- Reproducing a reported UI bug, then confirming the fix. Confirm the bug is
  real before changing anything; do not trust the diff alone.
- Before handing over a pull request that touches `web/`.

Not for changes confined to `sim-core`, `shells/`, or docs.

## What this needs

Browser automation: navigate, screenshot, read the console and network, click,
type, scroll, and evaluate a small expression in the page. **Any tool providing
those works** — this procedure names none, and nothing below depends on a
particular one.

Some agents have browser control built in. Others get it from an MCP server such
as `@playwright/mcp`; configure that at user scope, once, so it serves every
project rather than being pinned inside this repository. Playwright-backed tools
also need a browser binary, which is a separate install
(`npx playwright install chromium`).

If you have no browser tools, say so and stop. Do not install anything on the
basis of this file, and do not report a visual check you did not perform.

## How to run it

1. **Start the dev server** as `docs/development.md` describes. The `dev` script
   rebuilds the WASM bindings before Vite starts, so a cold first run is slow.
   Use the URL Vite prints; do not assume a port. Environment problems
   (a missing toolchain, `vite` not installed) are `docs/development.md`'s
   subject, not this file's.

2. **Load the page at a desktop viewport** — around 1200×1280 is a reasonable
   baseline. The status bar's `transport` readout shows `…` until the worker is
   ready; wait for it to name a transport before interacting. Then capture both:
   - an accessibility snapshot, for labels, values, disabled states and bounds;
   - a screenshot, for the canvas, overlays, clipping and hierarchy.

3. **Read the console and network.** Confirm the WASM module loaded. Report
   incidental failures separately and label them as such — a missing favicon is
   not an application or WebGL failure, and describing it as one wastes the
   reader's time.

4. **Exercise the behaviour under review.** That takes priority over everything
   below. For a broad smoke check the controls are play/pause, step, the speed
   buttons, the seed and founders inputs, reseed, copy link and reset view, plus
   clicking an agent to open the inspector and double-clicking to reset the view.

   Check what each control should *do* rather than a specific reading, since the
   numbers change as the simulation is tuned:
   - **Step** advances the tick while paused, and the chart and energy readouts follow.
   - **Play/pause** advances the tick continuously, disables step while running,
     and leaves the tick fixed once paused.
   - **Speed** changes how fast the tick advances and marks the active button.
   - **Reseed** resets the tick, reseeds the population, changes what is drawn,
     and updates the URL fragment to describe the run now on screen.
   - **Camera** — wheel zooms and the zoom readout follows; reset view returns to
     the fitted whole world.
   - **Inspector** — clicking a live agent opens it, and stepping updates its
     energy, age and activations. The genome disclosure renders and scrolls.
   - **Copy link** reports that it copied.

   Keep runs short. The Phase 1 economy can reach extinction quickly, and a
   population collapse is a simulation outcome, not a UI defect. Never substitute
   a screenshot for actually operating a control.

5. **Check a narrow viewport**, normally 390×844. A full-page screenshot can
   *hide* horizontal overflow by expanding to the document's scroll width, so
   compare `document.documentElement.scrollWidth` against `window.innerWidth`, or
   read element bounds. Report clipping, overlap, unreachable controls and
   horizontal scrolling even when the desktop layout is correct.

6. **Leave nothing behind.** Stop the dev server, close the page, and delete
   screenshots unless they were asked for. Confirm `git status` is clean — a
   validation run must not dirty the worktree.

## What this procedure does not cover

`web/vite.config.js` sets COOP/COEP on both `server` and `preview`, so
`crossOriginIsolated` is always true locally and `transport` always reads
`shared`. **The transferable fallback is never exercised this way**, and bugs
have lived there specifically — a frame handed back to the worker while the main
thread still held views over it made clicks silently miss, on that path only.

Say so in the report rather than implying both were covered. To cover it, serve
the built `web/dist` with a static server that does not set those headers and
repeat steps 2 to 4.

Nothing here runs in CI or fails a build, either. Anything this uncovers should
be encoded as a test before the branch lands, or the next change reintroduces it.

## Reporting

Lead with pass or fail. Separate desktop behaviour from responsive findings, and
separate a rendering or control defect from an expected simulation outcome. State
what was on screen, what you operated, the before and after, and any console or
network output verbatim. For a failure, give the viewport, the measured overflow
or bounds, and the exact error.

This is evidence for a human, not a verdict. `AGENTS.md` separates mechanical
checks from judgment and says the judgment half cannot be self-certified; a visual
check produces the observations, not the conclusion.
