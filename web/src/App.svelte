<script>
  // Cross-origin isolation is what SharedArrayBuffer actually gates on — checking the
  // response headers is not the same test (spec §7.7). The worker picks its transport
  // from this at startup; showing it here makes a broken deploy visible immediately.
  // Via globalThis: a bare reference is a ReferenceError, not `undefined`, on any
  // browser that predates the property — which would blank the page instead of
  // reporting the degraded transport.
  const isolated = globalThis.crossOriginIsolated === true;
</script>

<main>
  <h1>Synthetic Evolution</h1>
  <p>Scaffold. The sim worker and renderer arrive at M9.</p>
  <p class="status" class:ok={isolated}>
    crossOriginIsolated: {isolated} — snapshot transport will be
    {isolated ? 'SharedArrayBuffer (zero-copy)' : 'transferable ArrayBuffer (one copy/frame)'}
  </p>
</main>

<style>
  main {
    font: 15px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
    color: #d8dee9;
    background: #14161a;
    min-height: 100vh;
    margin: 0;
    padding: 2rem;
  }
  h1 { font-size: 1.25rem; font-weight: 600; }
  .status { color: #d08770; }
  .status.ok { color: #a3be8c; }
</style>
