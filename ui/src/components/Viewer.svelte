<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { EditorView } from "@codemirror/view";
  import { viewerState } from "../lib/editor/setup";

  let { doc, json }: { doc: string; json: boolean } = $props();
  let host: HTMLDivElement;
  let view: EditorView | undefined;

  onMount(() => {
    view = new EditorView({ parent: host, state: viewerState(doc, json) });
  });
  onDestroy(() => view?.destroy());

  $effect(() => {
    const d = doc;
    const j = json;
    if (view && view.state.doc.toString() !== d) view.setState(viewerState(d, j));
  });
</script>

<div class="viewer" bind:this={host}></div>

<style>
  .viewer {
    height: 100%;
    min-height: 0;
  }
</style>
