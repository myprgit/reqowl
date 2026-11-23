<script lang="ts">
  import type { Pair } from "../lib/api";

  interface Props {
    rows: Pair[];
    keyLabel?: string;
    valueLabel?: string;
    keyPlaceholder?: string;
    onchange: (rows: Pair[]) => void;
  }
  let { rows, keyLabel = "Name", valueLabel = "Value", keyPlaceholder = "name", onchange }: Props = $props();

  // Always show one empty row at the end to type into.
  const shown = $derived([...rows, { key: "", value: "" }]);

  function set(i: number, field: "key" | "value", v: string) {
    const next = rows.map((r) => ({ ...r }));
    if (i === rows.length) next.push({ key: "", value: "" });
    next[i][field] = v;
    onchange(next);
  }

  function remove(i: number) {
    onchange(rows.filter((_, j) => j !== i));
  }
</script>

<table class="kv">
  <thead>
    <tr><th>{keyLabel}</th><th>{valueLabel}</th><th class="x"></th></tr>
  </thead>
  <tbody>
    {#each shown as row, i (i)}
      <tr class:ghost={i === rows.length}>
        <td>
          <input
            value={row.key}
            placeholder={i === rows.length ? `Add ${keyPlaceholder}` : ""}
            spellcheck="false"
            oninput={(e) => set(i, "key", e.currentTarget.value)}
          />
        </td>
        <td>
          <input value={row.value} spellcheck="false" oninput={(e) => set(i, "value", e.currentTarget.value)} />
        </td>
        <td class="x">
          {#if i < rows.length}
            <button type="button" title="Remove" aria-label="Remove {row.key}" onclick={() => remove(i)}>×</button>
          {/if}
        </td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .kv {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
  }

  th {
    text-align: left;
    font-family: var(--font-ui);
    font-weight: 700;
    font-size: 12px;
    color: var(--ink-soft);
    padding: 4px 6px;
    border-bottom: 1px solid var(--rule-strong);
  }

  td {
    border-bottom: 1px solid var(--rule);
    padding: 0;
  }

  td:first-child {
    width: 36%;
    border-right: 1px solid var(--rule);
  }

  input {
    width: 100%;
    border: none;
    background: transparent;
    padding: 5px 6px;
    font: inherit;
    font-variation-settings: inherit;
  }

  input:focus {
    outline: none;
    background: var(--paper-raised);
    box-shadow: inset 0 -2px 0 var(--accent);
  }

  .ghost input::placeholder {
    color: var(--ink-faint);
  }

  .x {
    width: 28px;
    text-align: center;
  }

  .x button {
    border: none;
    background: transparent;
    color: var(--ink-faint);
    font-size: 15px;
    line-height: 1;
    padding: 2px 6px;
    border-radius: 4px;
  }

  .x button:hover {
    color: var(--err);
    background: var(--err-wash);
  }
</style>
