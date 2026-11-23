<script lang="ts">
  import { CODE_FONTS, NOTE_FONTS, type Settings, type Theme } from "../lib/themes";

  interface Props {
    settings: Settings;
    themes: Theme[];
    errors: string[];
    themesDir: string | null;
    onchange: (s: Settings) => void;
    onOpenFolder: () => void;
    onReload: () => void;
    onClose: () => void;
  }
  let p: Props = $props();

  const s = $derived(p.settings);
  const light = $derived(p.themes.filter((t) => !t.dark));
  const dark = $derived(p.themes.filter((t) => t.dark));

  function set<K extends keyof Settings>(key: K, value: Settings[K]) {
    p.onchange({ ...s, [key]: value });
  }

  function pick(t: Theme) {
    set("theme", t.id);
  }

  function chips(t: Theme) {
    return [t.colors.paper, t.colors.ink, t.colors.accent, t.colors.ok, t.colors.err];
  }

  function focus(el: HTMLElement) {
    el.focus();
  }
</script>

<div class="scrim" role="presentation" onclick={p.onClose}></div>
<div
  class="dialog"
  role="dialog"
  aria-modal="true"
  aria-label="Settings"
  tabindex="-1"
  use:focus
  onkeydown={(e) => e.key === "Escape" && p.onClose()}
>
  <header>
    <h1>Settings</h1>
    <button class="quiet-btn" onclick={p.onClose}>Done</button>
  </header>

  <div class="body">
    <section>
      <h2>Theme</h2>
      <label class="check">
        <input type="checkbox" checked={s.theme === "system"} onchange={(e) => set("theme", e.currentTarget.checked ? "system" : s.lightTheme)} />
        Match the system's light or dark mode
      </label>

      {#if s.theme === "system"}
        <div class="pair">
          <label>
            Light
            <select value={s.lightTheme} onchange={(e) => set("lightTheme", e.currentTarget.value)}>
              {#each light as t (t.id)}<option value={t.id}>{t.name}</option>{/each}
            </select>
          </label>
          <label>
            Dark
            <select value={s.darkTheme} onchange={(e) => set("darkTheme", e.currentTarget.value)}>
              {#each dark as t (t.id)}<option value={t.id}>{t.name}</option>{/each}
            </select>
          </label>
        </div>
      {:else}
        <div class="cards" role="radiogroup" aria-label="Theme">
          {#each p.themes as t (t.id)}
            <button
              class="card"
              role="radio"
              aria-checked={s.theme === t.id}
              class:on={s.theme === t.id}
              style:--c-paper={t.colors.paper}
              style:--c-ink={t.colors.ink}
              style:--c-rule={t.colors.rule}
              style:--c-accent={t.colors.accent}
              onclick={() => pick(t)}
            >
              <span class="sample">
                <span class="margin"></span>
                <span class="lines">Aa {"{{x}}"}</span>
              </span>
              <span class="name">{t.name}{#if t.file}<small> · custom</small>{/if}</span>
              <span class="chips">
                {#each chips(t) as c, i (i)}<i style:background={c}></i>{/each}
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </section>

    <section>
      <h2>Text</h2>
      <div class="grid">
        <label for="note-font">Notes</label>
        <div class="row">
          <select id="note-font" value={s.noteFont} onchange={(e) => set("noteFont", e.currentTarget.value)}>
            <option value="theme">Theme default</option>
            {#each NOTE_FONTS as f (f.id)}<option value={f.id}>{f.label}</option>{/each}
            <option value="custom">Another installed font…</option>
          </select>
          {#if s.noteFont === "custom"}
            <input placeholder="Font name, e.g. Inter" value={s.noteFontCustom} oninput={(e) => set("noteFontCustom", e.currentTarget.value)} />
          {/if}
        </div>

        <label for="code-font">Requests and responses</label>
        <div class="row">
          <select id="code-font" value={s.codeFont} onchange={(e) => set("codeFont", e.currentTarget.value)}>
            <option value="theme">Theme default</option>
            {#each CODE_FONTS as f (f.id)}<option value={f.id}>{f.label}</option>{/each}
            <option value="custom">Another installed font…</option>
          </select>
          {#if s.codeFont === "custom"}
            <input placeholder="Font name, e.g. JetBrains Mono" value={s.codeFontCustom} oninput={(e) => set("codeFontCustom", e.currentTarget.value)} />
          {/if}
        </div>

        <label for="size">Size</label>
        <div class="row">
          <input id="size" type="range" min="12" max="20" step="1" value={s.fontSize} oninput={(e) => set("fontSize", Number(e.currentTarget.value))} />
          <span class="val">{s.fontSize}px</span>
        </div>

        <span></span>
        <label class="check">
          <input type="checkbox" checked={s.ruled} onchange={(e) => set("ruled", e.currentTarget.checked)} />
          Ruled lines on the page
        </label>
      </div>
    </section>

    <section>
      <h2>Your own themes</h2>
      <p class="hint">
        A theme is a small TOML file in the themes folder. The folder has a commented example,
        <code>_example.toml</code>; copy it to a new name, change the colours, then reload.
      </p>
      {#if p.themesDir}<p class="path">{p.themesDir}</p>{/if}
      <div class="row">
        <button class="quiet-btn bordered" onclick={p.onOpenFolder}>Open themes folder</button>
        <button class="quiet-btn bordered" onclick={p.onReload}>Reload themes</button>
      </div>
      {#each p.errors as err (err)}
        <p class="error">{err}</p>
      {/each}
    </section>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--ink) 28%, transparent);
    z-index: 60;
  }

  .dialog {
    position: fixed;
    top: 6vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(720px, 94vw);
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    background: var(--paper);
    color: var(--ink);
    border: 1px solid var(--rule-strong);
    border-radius: 10px;
    z-index: 61;
    box-shadow: 0 20px 60px color-mix(in srgb, var(--ink) 25%, transparent);
    outline: none;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 18px;
    border-bottom: 1px solid var(--rule-strong);
  }

  h1 {
    font-size: 17px;
    margin: 0;
  }

  .body {
    overflow: auto;
    padding: 6px 18px 18px;
  }

  section {
    padding: 12px 0 14px;
    border-bottom: 1px dashed var(--rule-strong);
  }

  section:last-child {
    border-bottom: none;
  }

  h2 {
    font-size: 14px;
    margin: 0 0 10px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13.5px;
  }

  .pair {
    display: flex;
    gap: 18px;
    margin-top: 10px;
  }

  .pair label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--ink-soft);
  }

  select,
  input:not([type]) {
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    background: var(--paper-raised);
    color: var(--ink);
    padding: 4px 8px;
    font-size: 13px;
  }

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px;
    margin-top: 10px;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px;
    border: 1px solid var(--rule-strong);
    border-radius: 8px;
    background: var(--paper-raised);
    text-align: left;
  }

  .card.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent);
  }

  .sample {
    position: relative;
    height: 54px;
    border-radius: 5px;
    background-color: var(--c-paper);
    background-image: linear-gradient(to bottom, transparent 13px, var(--c-rule) 13px);
    background-size: 100% 14px;
    overflow: hidden;
  }

  .margin {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 18px;
    width: 4px;
    background: linear-gradient(to right, var(--c-accent) 0 1.5px, transparent 1.5px 3px, var(--c-accent) 3px 4px);
  }

  .lines {
    position: absolute;
    left: 28px;
    top: 14px;
    color: var(--c-ink);
    font-family: var(--font-code);
    font-size: 15px;
    font-weight: 700;
  }

  .name {
    font-size: 13px;
    font-weight: 700;
  }

  .name small {
    font-weight: 400;
    color: var(--ink-faint);
  }

  .chips {
    display: flex;
    gap: 3px;
  }

  .chips i {
    width: 14px;
    height: 8px;
    border-radius: 2px;
    border: 1px solid color-mix(in srgb, var(--ink) 15%, transparent);
  }

  .grid {
    display: grid;
    grid-template-columns: 180px 1fr;
    gap: 10px 14px;
    align-items: center;
  }

  .grid > label {
    font-size: 13px;
    color: var(--ink-soft);
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }

  .row input[type="range"] {
    width: 200px;
    accent-color: var(--accent);
  }

  .val {
    font-size: 13px;
    color: var(--ink-soft);
    min-width: 40px;
  }

  .bordered {
    border-color: var(--rule-strong);
    color: var(--ink);
  }

  .hint {
    font-size: 13px;
    color: var(--ink-soft);
    margin: 0 0 8px;
    max-width: 60ch;
  }

  .path {
    font-family: var(--font-code);
    font-size: 12px;
    color: var(--ink-faint);
    margin: 0 0 8px;
    word-break: break-all;
  }

  code {
    background: var(--code-bg);
    padding: 0 4px;
    border-radius: 3px;
    font-family: var(--font-code);
    font-size: 12px;
  }

  .error {
    color: var(--err);
    font-size: 12.5px;
    margin: 8px 0 0;
  }
</style>
