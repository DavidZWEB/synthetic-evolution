<script lang="ts">
  import { onMount } from 'svelte';
  import type { MetricSample } from '../telemetry/history';

  let { samples }: { samples: MetricSample[] } = $props();
  let canvas: HTMLCanvasElement;

  function plot(
    context: CanvasRenderingContext2D,
    values: number[],
    top: number,
    height: number,
    width: number,
    color: string,
  ) {
    const maximum = Math.max(1, ...values);
    context.strokeStyle = '#2b2f38';
    context.beginPath();
    context.moveTo(0, top + height);
    context.lineTo(width, top + height);
    context.stroke();

    if (values.length < 2) return;
    context.strokeStyle = color;
    context.lineWidth = 1.5;
    context.beginPath();
    for (let index = 0; index < values.length; index += 1) {
      const x = (index / (values.length - 1)) * width;
      const y = top + height - (values[index] / maximum) * height;
      if (index === 0) context.moveTo(x, y);
      else context.lineTo(x, y);
    }
    context.stroke();
  }

  function draw(current: MetricSample[]) {
    if (!canvas) return;
    const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
    const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
    const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }

    const context = canvas.getContext('2d');
    if (!context) return;
    context.clearRect(0, 0, width, height);
    const gap = 8 * ratio;
    const panelHeight = (height - gap) / 2;
    plot(context, current.map((sample) => sample.population), 0, panelHeight, width, '#a3be8c');
    plot(
      context,
      current.map((sample) => sample.meanEnergy),
      panelHeight + gap,
      panelHeight,
      width,
      '#88c0d0',
    );
  }

  $effect(() => {
    draw(samples);
  });

  onMount(() => {
    const observer = new ResizeObserver(() => draw(samples));
    observer.observe(canvas);
    return () => observer.disconnect();
  });
</script>

<section class="telemetry" aria-label="Live population and energy history">
  <header>
    <span class="population">population {samples.at(-1)?.population ?? 0}</span>
    <span class="energy">mean energy {(samples.at(-1)?.meanEnergy ?? 0).toFixed(1)}</span>
  </header>
  <canvas bind:this={canvas}></canvas>
</section>

<style>
  .telemetry {
    position: absolute;
    left: 0.75rem;
    bottom: 0.75rem;
    width: min(28rem, calc(100% - 1.5rem));
    padding: 0.55rem;
    box-sizing: border-box;
    border: 1px solid #2b2f38;
    border-radius: 4px;
    background: rgb(14 16 20 / 88%);
    pointer-events: none;
  }

  header {
    display: flex;
    gap: 1rem;
    margin-bottom: 0.25rem;
    font-size: 0.72rem;
  }

  .population { color: #a3be8c; }
  .energy { color: #88c0d0; }

  canvas {
    display: block;
    width: 100%;
    height: 5rem;
  }
</style>
