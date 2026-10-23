<script lang="ts">
  import { tick } from 'svelte';
  import { app } from '../lib/state.svelte';

  const dialog = $derived(app.dialog);
  let value = $state('');
  let input = $state<HTMLInputElement>();
  let primary = $state<HTMLButtonElement>();

  $effect(() => {
    const spec = dialog?.spec;
    if (!spec) return;
    if (spec.type === 'prompt') value = spec.value;
    void tick().then(() => {
      if (spec.type === 'prompt' && input) {
        input.focus();
        // Select the name without its extension, like Finder.
        const dot = spec.selectStem ? spec.value.lastIndexOf('.') : -1;
        input.setSelectionRange(0, dot > 0 ? dot : spec.value.length);
      } else {
        primary?.focus();
      }
    });
  });

  function close(result: string | null) {
    dialog?.resolve(result);
  }

  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') close(null);
    if (e.key === 'Enter' && dialog?.spec.type === 'prompt') {
      e.preventDefault();
      if (value.trim()) close(value.trim());
    }
  }
</script>

{#if dialog}
  {@const spec = dialog.spec}
  <div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close(null)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title" tabindex="-1" onkeydown={onKey}>
      <h2 id="dialog-title">{spec.title}</h2>

      {#if spec.type === 'prompt'}
        <label>
          <span>{spec.label}</span>
          <input class="field" bind:this={input} bind:value spellcheck="false" />
        </label>
        <div class="buttons">
          <button class="btn" onclick={() => close(null)}>Cancel</button>
          <button class="btn primary" bind:this={primary} disabled={!value.trim()} onclick={() => close(value.trim())}>{spec.confirm}</button>
        </div>
      {:else if spec.type === 'confirm'}
        <p>{spec.message}</p>
        <div class="buttons">
          <button class="btn" onclick={() => close(null)}>Cancel</button>
          <button class="btn primary" class:danger={spec.danger} bind:this={primary} onclick={() => close('ok')}>{spec.confirm}</button>
        </div>
      {:else}
        <p>{spec.message}</p>
        {#if spec.details}
          <ul class="details">
            {#each spec.details as d (d)}<li>{d}</li>{/each}
          </ul>
        {/if}
        <div class="buttons">
          {#each spec.choices as choice (choice.value)}
            {#if choice.primary}
              <button class="btn primary" bind:this={primary} onclick={() => close(choice.value)}>{choice.label}</button>
            {:else}
              <button class="btn" class:danger={choice.danger} onclick={() => close(choice.value)}>{choice.label}</button>
            {/if}
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: start center;
    padding-top: 18vh;
    background: rgba(10, 14, 24, 0.28);
  }
  .dialog {
    width: min(420px, calc(100vw - 32px));
    padding: 18px 20px 16px;
    border-radius: 12px;
    background: var(--raised);
    box-shadow: var(--shadow);
    border: 1px solid var(--border);
    outline: none;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 15px;
  }
  p {
    margin: 0 0 12px;
    color: var(--muted);
  }
  label {
    display: grid;
    gap: 6px;
    margin: 12px 0;
    color: var(--muted);
    font-size: 12px;
  }
  .details {
    margin: 0 0 12px;
    padding-left: 18px;
    max-height: 120px;
    overflow: auto;
    font-size: 12px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
