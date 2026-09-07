<script>
  const SPEEDS = [1, 2, 5, 10, 25, 50, 100];

  let {
    ready,
    running,
    speed,
    seed,
    founders,
    brainInheritance,
    shareUrl,
    linkCopied,
    ontoggle,
    onstep,
    onspeed,
    onseed,
    onfounders,
    onbraininheritance,
    onreseed,
    oncopy,
    onreset,
  } = $props();
</script>

<footer>
  <button onclick={ontoggle} disabled={!ready}>{running ? 'pause' : 'play'}</button>
  <button onclick={onstep} disabled={!ready || running}>step</button>

  <span class="speeds">
    {#each SPEEDS as option}
      <button class:active={speed === option} onclick={() => onspeed(option)}>
        {option}×
      </button>
    {/each}
  </span>

  <label>
    seed
    <input
      type="text"
      inputmode="numeric"
      pattern="[0-9]*"
      value={seed}
      oninput={(event) => onseed(event.currentTarget.value)}
    />
  </label>
  <label>
    founders
    <input
      type="number"
      value={founders}
      min="1"
      oninput={(event) => onfounders(event.currentTarget.valueAsNumber)}
    />
  </label>
  <label>
    heredity
    <select
      value={brainInheritance}
      onchange={(event) => onbraininheritance(event.currentTarget.value)}
    >
      <option value="evolving">evolving</option>
      <option value="randomized_at_birth">random control</option>
    </select>
  </label>
  <button onclick={onreseed}>reseed</button>
  <button onclick={oncopy} disabled={!ready || !shareUrl}>
    {linkCopied ? 'copied' : 'copy link'}
  </button>
  <button onclick={onreset}>reset view</button>
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.6rem 1rem;
    border-top: 1px solid #23262d;
    background: #14161a;
    flex-wrap: wrap;
  }

  button {
    font: inherit;
    color: #d8dee9;
    background: #1c1f26;
    border: 1px solid #2b2f38;
    border-radius: 3px;
    padding: 0.25rem 0.6rem;
    cursor: pointer;
  }

  button:hover:not(:disabled) { background: #242832; }
  button:disabled { opacity: 0.4; cursor: default; }
  button.active { background: #3b4252; border-color: #4c566a; }

  .speeds { display: flex; flex-wrap: wrap; min-width: 0; gap: 0.25rem; }

  label {
    color: #6b7280;
    display: flex;
    gap: 0.35rem;
    align-items: center;
  }

  input,
  select {
    font: inherit;
    color: #d8dee9;
    background: #1c1f26;
    border: 1px solid #2b2f38;
    border-radius: 3px;
    padding: 0.2rem 0.4rem;
  }

  input { width: 5rem; }

  @media (max-width: 40rem) {
    footer { gap: 0.5rem; padding: 0.6rem 0.75rem; }
    input { width: 4rem; }
  }
</style>
