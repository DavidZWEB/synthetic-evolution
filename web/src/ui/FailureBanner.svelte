<script>
  let { message, dismissible = false, ondismiss = () => {} } = $props();
</script>

<div class="failure" role="alert">
  <span>{message}</span>
  {#if dismissible}<button onclick={ondismiss}>dismiss</button>{/if}
</div>

<style>
  .failure {
    position: absolute;
    top: 1rem;
    right: 1rem;
    left: 1rem;
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.6rem;
    background: rgb(14 16 20 / 92%);
    border: 1px solid #bf616a;
    color: #bf616a;
    pointer-events: none;
  }

  button {
    border: 0;
    padding: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
    pointer-events: auto;
  }
</style>
