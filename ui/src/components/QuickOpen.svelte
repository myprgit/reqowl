<script lang="ts">
  import type { TreeNode } from "../lib/api";

  let { pages, onPick, onClose }: { pages: TreeNode[]; onPick: (id: string) => void; onClose: () => void } = $props();
  let q = $state("");
  let sel = $state(0);

  const matches = $derived.by(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return pages.slice(0, 50);
    return pages
      .map((p) => ({ p, i: p.id.toLowerCase().indexOf(needle), t: p.title.toLowerCase().indexOf(needle) }))
      .filter((m) => m.i >= 0)
      .sort((a, b) => (a.t >= 0 ? a.t : 99) - (b.t >= 0 ? b.t : 99) || a.p.id.length - b.p.id.length)
      .slice(0, 50)
      .map((m) => m.p);
  });

  $effect(() => {
    void q;
    sel = 0;
  });

  function key(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      sel = Math.min(sel + 1, matches.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === "Enter" && matches[sel]) {
      onPick(matches[sel].id);
    } else if (e.key === "Escape") {
      onClose();
    }
  }

  function focus(el: HTMLInputElement) {
    el.focus();
  }
</script>

<div class="scrim" role="presentation" onclick={onClose}></div>
<div class="box" role="dialog" aria-label="Go to page">
  <input bind:value={q} use:focus onkeydown={key} placeholder="Go to page" />
  <ul>
    {#each matches as m, i (m.id)}
      <li>
        <button class:on={i === sel} onclick={() => onPick(m.id)} onmouseenter={() => (sel = i)}>
          <span class="t">{m.title}</span>
          {#if m.id.includes("/")}<span class="path">{m.id.slice(0, m.id.lastIndexOf("/"))}</span>{/if}
        </button>
      </li>
    {/each}
    {#if matches.length === 0}
      <li class="none">No page matches “{q}”.</li>
    {/if}
  </ul>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--ink) 22%, transparent);
    z-index: 40;
  }

  .box {
    position: fixed;
    top: 14%;
    left: 50%;
    transform: translateX(-50%);
    width: min(520px, 90vw);
    background: var(--paper-raised);
    border: 1px solid var(--rule-strong);
    border-radius: 10px;
    z-index: 41;
    overflow: hidden;
    box-shadow: 0 18px 50px color-mix(in srgb, var(--ink) 25%, transparent);
  }

  input {
    width: 100%;
    border: none;
    border-bottom: 1px solid var(--rule-strong);
    background: transparent;
    padding: 12px 16px;
    font-size: 16px;
    outline: none;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 50vh;
    overflow: auto;
  }

  button {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    padding: 6px 12px;
    border-radius: 6px;
  }

  button.on {
    background: var(--amber-wash);
  }

  .path {
    color: var(--ink-faint);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .none {
    padding: 10px 12px;
    color: var(--ink-faint);
  }
</style>
