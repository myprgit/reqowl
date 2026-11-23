<script lang="ts">
  interface Props {
    recent: string[];
    error: string | null;
    onOpen: () => void;
    onCreate: () => void;
    onImport: () => void;
    onOpenRecent: (path: string) => void;
  }
  let p: Props = $props();
</script>

<div class="welcome">
  <div class="sheet">
    <img src="/owl.png" alt="The reqowl owl, reading a notebook" width="148" height="148" />
    <h1>reqowl</h1>
    <p class="lede">Notes and API requests on the same page. A notebook is a folder of Markdown files you can keep in git.</p>

    <div class="actions">
      <button class="ink-btn" onclick={p.onOpen}>Open a notebook</button>
      <button class="quiet-btn" onclick={p.onCreate}>Create a notebook</button>
      <button class="quiet-btn" onclick={p.onImport}>Import from Postman or Insomnia</button>
    </div>

    {#if p.error}
      <p class="error" role="alert">{p.error}</p>
    {/if}

    {#if p.recent.length}
      <h2>Recent</h2>
      <ul>
        {#each p.recent as path (path)}
          <li><button onclick={() => p.onOpenRecent(path)} title={path}>{path}</button></li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .welcome {
    height: 100%;
    display: grid;
    place-items: center;
    overflow: auto;
    background-image: linear-gradient(to bottom, transparent calc(var(--line) - 1px), var(--rule) calc(var(--line) - 1px));
    background-size: 100% var(--line);
  }

  .sheet {
    width: min(560px, 92vw);
    padding: 34px 40px 30px 64px;
    background: var(--paper-raised);
    border: 1px solid var(--rule-strong);
    border-radius: 4px;
    position: relative;
    box-shadow: 0 14px 40px color-mix(in srgb, var(--ink) 12%, transparent);
  }

  /* Same amber margin the editor uses. */
  .sheet::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 40px;
    width: 6px;
    background: linear-gradient(to right, var(--amber) 0 2px, transparent 2px 5px, var(--amber) 5px 6px);
  }

  img {
    display: block;
    margin-bottom: 4px;
  }

  h1 {
    font-family: var(--font-note);
    font-variation-settings: "MONO" 0, "CASL" 1, "slnt" 0, "CRSV" 1;
    font-size: 40px;
    line-height: 1;
    margin: 0 0 10px;
    letter-spacing: -0.02em;
  }

  .lede {
    color: var(--ink-soft);
    font-size: 15px;
    max-width: 44ch;
    margin: 0 0 22px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .actions .quiet-btn {
    border-color: var(--rule-strong);
    color: var(--ink);
  }

  .error {
    margin-top: 16px;
    color: var(--err);
  }

  h2 {
    font-size: 13px;
    color: var(--ink-soft);
    margin: 26px 0 6px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li button {
    border: none;
    background: transparent;
    padding: 3px 0;
    color: var(--ink);
    text-align: left;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  li button:hover {
    color: var(--amber-ink);
    text-decoration: underline;
  }
</style>
