<script lang="ts">
  import type { WorkspaceInfo } from "../lib/api";
  import TreeItem from "./TreeItem.svelte";

  type Doc = { kind: "page"; id: string } | { kind: "text"; rel: string };

  interface Props {
    ws: WorkspaceInfo;
    current: Doc | null;
    onSelectPage: (id: string) => void;
    onSelectText: (rel: string) => void;
    onCreate: (parent: string | null) => void;
    onRename: (id: string, title: string) => void;
    onMove: (id: string, parent: string | null, before: string | null) => void;
    onTrash: (id: string) => void;
    onCreateEnv: () => void;
    onImport: () => void;
    onSwitch: () => void;
  }
  let p: Props = $props();

  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  let renaming = $state<string | null>(null);
  let dropRoot = $state(false);

  const currentId = $derived(p.current?.kind === "page" ? p.current.id : null);
  const currentRel = $derived(p.current?.kind === "text" ? p.current.rel : null);

  function openMenu(id: string, e: MouseEvent) {
    e.preventDefault();
    menu = { id, x: e.clientX, y: e.clientY };
  }

  function onRootDrop(e: DragEvent) {
    e.preventDefault();
    dropRoot = false;
    const id = e.dataTransfer?.getData("application/x-reqowl-page");
    if (id && id.includes("/")) p.onMove(id, null, null);
  }
</script>

<svelte:window onclick={() => (menu = null)} />

<aside>
  <button class="book" onclick={p.onSwitch} title="{p.ws.root}&#10;Click to open another notebook">
    <img src="/owl.png" alt="" width="26" height="26" />
    <span>{p.ws.name}</span>
  </button>

  <nav
    class="tree"
    class:drop={dropRoot}
    aria-label="Pages"
    ondragover={(e) => {
      e.preventDefault();
      dropRoot = true;
    }}
    ondragleave={() => (dropRoot = false)}
    ondrop={onRootDrop}
  >
    {#each p.ws.tree as node (node.id)}
      <TreeItem
        {node}
        depth={0}
        {currentId}
        bind:renaming
        onSelect={p.onSelectPage}
        onMenu={openMenu}
        onRename={p.onRename}
        onMove={p.onMove}
      />
    {/each}
    <button class="add" onclick={() => p.onCreate(null)} title="Ctrl+N">+ New page</button>
  </nav>

  <section class="settings">
    <h2>Environments</h2>
    {#each p.ws.envs as name (name)}
      <button class="row" class:active={currentRel === `env/${name}.toml`} onclick={() => p.onSelectText(`env/${name}.toml`)}>
        {name}{#if name === p.ws.default_env}<span class="tag">default</span>{/if}
      </button>
      <button
        class="row sub"
        class:active={currentRel === `env/${name}.local.toml`}
        onclick={() => p.onSelectText(`env/${name}.local.toml`)}
        title="Secrets for {name}. This file stays out of git and its values are hidden in history."
      >
        {name} secrets
      </button>
    {/each}
    <button class="add" onclick={p.onCreateEnv}>+ New environment</button>
    <div class="foot">
      <button class="row" class:active={currentRel === "reqowl.toml"} onclick={() => p.onSelectText("reqowl.toml")}>
        Notebook settings
      </button>
      <button class="row" onclick={p.onImport}>Import Postman or Insomnia…</button>
    </div>
  </section>
</aside>

{#if menu}
  {@const id = menu.id}
  <div class="menu" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    <button role="menuitem" onclick={() => p.onCreate(id)}>New page inside</button>
    <button role="menuitem" onclick={() => (renaming = id)}>Rename</button>
    <button role="menuitem" class="danger" onclick={() => p.onTrash(id)}>Move to trash</button>
  </div>
{/if}

<style>
  aside {
    background: var(--paper-side);
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }

  .book {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 9px 14px;
    min-height: 44px;
    border: none;
    border-bottom: 1px solid var(--rule-strong);
    background: transparent;
    text-align: left;
    font-family: var(--font-note);
    font-variation-settings: "MONO" 0, "CASL" 1, "slnt" 0, "CRSV" 1;
    font-weight: 700;
    font-size: 15px;
  }

  .book span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .book:hover {
    background: var(--paper);
  }

  .tree {
    flex: 1;
    overflow: auto;
    padding: 8px 6px;
    border: 2px solid transparent;
  }

  .tree.drop {
    border-color: color-mix(in srgb, var(--amber) 50%, transparent);
  }

  .add {
    display: block;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    color: var(--ink-faint);
    padding: 4px 10px;
    border-radius: 5px;
    margin-top: 2px;
  }

  .add:hover {
    color: var(--ink);
    background: var(--paper);
  }

  .settings {
    border-top: 1px solid var(--rule-strong);
    padding: 8px 6px 10px;
    max-height: 45%;
    overflow: auto;
  }

  h2 {
    font-size: 12px;
    font-weight: 700;
    color: var(--ink-soft);
    margin: 2px 10px 4px;
  }

  .row {
    display: flex;
    gap: 6px;
    align-items: baseline;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 3px 10px;
    border-radius: 5px;
    font-size: 13px;
  }

  .row.sub {
    padding-left: 22px;
    color: var(--ink-soft);
    font-size: 12px;
  }

  .row:hover {
    background: var(--paper);
  }

  .row.active {
    background: var(--amber-wash);
  }

  .tag {
    font-size: 11px;
    color: var(--ink-faint);
  }

  .foot {
    margin-top: 8px;
    padding-top: 6px;
    border-top: 1px dashed var(--rule-strong);
  }

  .menu {
    position: fixed;
    z-index: 30;
    background: var(--paper-raised);
    border: 1px solid var(--rule-strong);
    border-radius: 8px;
    padding: 4px;
    min-width: 170px;
    box-shadow: 0 8px 22px color-mix(in srgb, var(--ink) 18%, transparent);
  }

  .menu button {
    display: block;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 5px 10px;
    border-radius: 5px;
  }

  .menu button:hover {
    background: var(--amber-wash);
  }

  .menu .danger {
    color: var(--err);
  }
</style>
