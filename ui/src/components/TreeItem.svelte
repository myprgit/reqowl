<script lang="ts">
  import type { TreeNode } from "../lib/api";
  import Self from "./TreeItem.svelte";

  interface Props {
    node: TreeNode;
    depth: number;
    currentId: string | null;
    renaming: string | null;
    onSelect: (id: string) => void;
    onMenu: (id: string, e: MouseEvent) => void;
    onRename: (id: string, title: string) => void;
    onMove: (id: string, parent: string | null, before: string | null) => void;
  }
  let { node, depth, currentId, renaming = $bindable(), onSelect, onMenu, onRename, onMove }: Props = $props();

  const storeKey = () => `reqowl.open.${node.id}`;
  let open = $state(localStorage.getItem(storeKey()) !== "0");
  let drop = $state<"before" | "inside" | null>(null);
  let draft = $state("");

  const parentId = $derived(node.id.includes("/") ? node.id.slice(0, node.id.lastIndexOf("/")) : null);

  // Keep the branch with the open page expanded.
  $effect(() => {
    if (currentId?.startsWith(node.id + "/")) open = true;
  });

  function toggle(e: MouseEvent) {
    e.stopPropagation();
    open = !open;
    localStorage.setItem(storeKey(), open ? "1" : "0");
  }

  function startRename() {
    draft = node.title;
    renaming = node.id;
  }

  $effect(() => {
    if (renaming === node.id && !draft) draft = node.title;
  });

  function finishRename(commit: boolean) {
    const t = draft.trim();
    renaming = null;
    draft = "";
    if (commit && t && t !== node.title) onRename(node.id, t);
  }

  function onDragOver(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    drop = e.clientY < r.top + r.height * 0.3 ? "before" : "inside";
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    const id = e.dataTransfer?.getData("application/x-reqowl-page");
    const where = drop;
    drop = null;
    if (!id || id === node.id) return;
    if (where === "before") onMove(id, parentId, node.title);
    else {
      onMove(id, node.id, null);
      open = true;
    }
  }

  function focusInput(el: HTMLInputElement) {
    el.focus();
    el.select();
  }
</script>

<div
  class="item"
  class:current={currentId === node.id}
  class:drop-before={drop === "before"}
  class:drop-inside={drop === "inside"}
  style:padding-left="{8 + depth * 14}px"
  draggable={renaming !== node.id}
  role="treeitem"
  aria-selected={currentId === node.id}
  aria-expanded={node.children.length ? open : undefined}
  tabindex="0"
  ondragstart={(e) => e.dataTransfer?.setData("application/x-reqowl-page", node.id)}
  ondragover={onDragOver}
  ondragleave={() => (drop = null)}
  ondrop={onDrop}
  onclick={() => onSelect(node.id)}
  ondblclick={startRename}
  oncontextmenu={(e) => onMenu(node.id, e)}
  onkeydown={(e) => {
    if (e.key === "Enter") onSelect(node.id);
    if (e.key === "F2") startRename();
  }}
>
  {#if node.children.length}
    <button class="twisty" tabindex="-1" onclick={toggle} aria-label={open ? "Collapse" : "Expand"}>{open ? "▾" : "▸"}</button>
  {:else}
    <span class="twisty leaf">·</span>
  {/if}
  {#if renaming === node.id}
    <input
      class="rename"
      bind:value={draft}
      use:focusInput
      onclick={(e) => e.stopPropagation()}
      onblur={() => finishRename(true)}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finishRename(true);
        if (e.key === "Escape") finishRename(false);
      }}
    />
  {:else}
    <span class="title" class:folder={!node.has_page}>{node.title}</span>
  {/if}
</div>

{#if open}
  {#each node.children as child (child.id)}
    <Self node={child} depth={depth + 1} {currentId} bind:renaming {onSelect} {onMenu} {onRename} {onMove} />
  {/each}
{/if}

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 8px;
    border-radius: 5px;
    cursor: default;
    user-select: none;
    border-top: 2px solid transparent;
  }

  .item:hover {
    background: var(--paper);
  }

  .item.current {
    background: var(--amber-wash);
    box-shadow: inset 3px 0 0 var(--amber);
  }

  .item.drop-before {
    border-top-color: var(--amber);
  }

  .item.drop-inside {
    outline: 1px dashed var(--amber);
  }

  .twisty {
    width: 16px;
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--ink-faint);
    padding: 0;
    font-size: 11px;
    text-align: center;
  }

  .leaf {
    color: var(--rule-strong);
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13.5px;
  }

  .title.folder {
    color: var(--ink-soft);
    font-style: italic;
  }

  .rename {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    border: 1px solid var(--amber);
    border-radius: 4px;
    background: var(--paper);
    padding: 1px 4px;
  }
</style>
