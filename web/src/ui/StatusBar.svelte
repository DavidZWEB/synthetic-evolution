<script>
  let { tick, population, meanEnergy, fps, zoom, transport } = $props();
</script>

<header>
  <h1>Synthetic Evolution</h1>
  <dl>
    <div><dt>tick</dt><dd>{tick}</dd></div>
    <div><dt>agents</dt><dd>{population}</dd></div>
    <div><dt>mean energy</dt><dd>{meanEnergy.toFixed(1)}</dd></div>
    <div><dt>fps</dt><dd class:slow={fps > 0 && fps < 55}>{fps}</dd></div>
    <div><dt>zoom</dt><dd>{zoom < 10 ? zoom.toFixed(1) : Math.round(zoom)}×</dd></div>
    <div>
      <dt>transport</dt>
      <dd class:degraded={transport === 'transferable'}>{transport ?? '…'}</dd>
    </div>
  </dl>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.6rem 1rem;
    background: #14161a;
    border-bottom: 1px solid #23262d;
  }

  h1 {
    font-size: 0.95rem;
    font-weight: 600;
    margin: 0;
    letter-spacing: 0.02em;
  }

  dl {
    display: flex;
    gap: 1.25rem;
    margin: 0 0 0 auto;
  }

  dl div { display: flex; gap: 0.4rem; }
  dt { color: #6b7280; }
  dd { margin: 0; color: #a3be8c; font-variant-numeric: tabular-nums; }
  dd.slow, dd.degraded { color: #d08770; }
</style>
