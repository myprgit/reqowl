<script lang="ts">
  import type { Auth, BodyMode, Pair, RequestForm } from "../lib/api";
  import KeyValues from "./KeyValues.svelte";

  interface Props {
    form: RequestForm;
    error: string | null;
    running: boolean;
    onchange: (form: RequestForm) => void;
    onsend: () => void;
    /** True while the user is typing in the form, so outside updates wait. */
    focused?: boolean;
  }
  let { form, error, running, onchange, onsend, focused = $bindable(false) }: Props = $props();

  type Section = "params" | "headers" | "auth" | "body" | "settings";
  let section = $state<Section>("params");

  const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
  const isWs = $derived(form.kind === "websocket");
  const isGql = $derived(form.kind === "graphql");
  const named = (rows: Pair[]) => rows.filter((r) => r.key.trim()).length;

  function update(patch: Partial<RequestForm>) {
    onchange({ ...form, ...patch });
  }

  function setAuth(type: Auth["type"]) {
    const next: Auth =
      type === "bearer"
        ? { type, token: "" }
        : type === "basic"
          ? { type, username: "", password: "" }
          : type === "apikey"
            ? { type, header: "X-API-Key", value: "" }
            : type === "oauth2"
              ? { type, profile: "" }
              : { type: "none" };
    update({ auth: next });
  }

  function setAuthField(field: string, value: string) {
    update({ auth: { ...form.auth, [field]: value } as Auth });
  }

  function setBodyMode(mode: BodyMode) {
    update({ body_mode: mode });
  }

  function formatJson() {
    try {
      update({ body: JSON.stringify(JSON.parse(form.body), null, 2) });
    } catch {
      /* leave invalid JSON (it may hold {{variables}}) as typed */
    }
  }

  function onUrlPaste(e: ClipboardEvent) {
    // Pasting a full URL with a query string fills the params table.
    const t = e.clipboardData?.getData("text/plain")?.trim() ?? "";
    const q = t.indexOf("?");
    if (q < 0 || /\s/.test(t)) return;
    e.preventDefault();
    const params = t
      .slice(q + 1)
      .split("&")
      .filter(Boolean)
      .map((p) => {
        const eq = p.indexOf("=");
        return eq < 0 ? { key: p, value: "" } : { key: p.slice(0, eq), value: p.slice(eq + 1) };
      });
    update({ url: t.slice(0, q), params: [...form.params, ...params] });
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="req"
  onfocusin={() => (focused = true)}
  onfocusout={() => (focused = false)}
  onkeydown={(e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      onsend();
    }
  }}
>
  <div class="line">
    {#if isWs}
      <span class="kind">WS</span>
    {:else if isGql}
      <span class="kind">GraphQL</span>
    {:else}
      <select class="method" value={form.method} onchange={(e) => update({ method: e.currentTarget.value })} aria-label="Method">
        {#each METHODS as m (m)}
          <option value={m}>{m}</option>
        {/each}
        {#if !METHODS.includes(form.method)}<option value={form.method}>{form.method}</option>{/if}
      </select>
    {/if}
    <input
      class="url"
      value={form.url}
      placeholder={"{{base}}/path"}
      spellcheck="false"
      aria-label="URL"
      oninput={(e) => update({ url: e.currentTarget.value })}
      onpaste={onUrlPaste}
    />
    <button class="ink-btn send" onclick={onsend} disabled={running} title="Ctrl+Enter">
      {running ? "Sending…" : isWs ? "Connect" : "Send"}
    </button>
  </div>

  <nav class="sections" aria-label="Request parts">
    {#if !isWs}
      <button class:on={section === "params"} onclick={() => (section = "params")}>
        Params{#if named(form.params)}<span class="n">{named(form.params)}</span>{/if}
      </button>
    {/if}
    <button class:on={section === "headers"} onclick={() => (section = "headers")}>
      Headers{#if named(form.headers)}<span class="n">{named(form.headers)}</span>{/if}
    </button>
    <button class:on={section === "auth"} onclick={() => (section = "auth")}>
      Auth{#if form.auth.type !== "none"}<span class="n">{form.auth.type}</span>{/if}
    </button>
    <button class:on={section === "body"} onclick={() => (section = "body")}>
      {isWs ? "Messages" : isGql ? "Query" : "Body"}{#if form.body_mode !== "none" && !isGql}<span class="n">{form.body_mode}</span>{/if}
    </button>
    <button class:on={section === "settings"} onclick={() => (section = "settings")}>Settings</button>
  </nav>

  <div class="section">
    {#if section === "params" && !isWs}
      <KeyValues rows={form.params} keyPlaceholder="parameter" onchange={(params) => update({ params })} />
      <p class="hint">Sent in the URL. Values can use <code>{"{{variables}}"}</code>.</p>
    {:else if section === "headers"}
      <KeyValues rows={form.headers} keyPlaceholder="header" onchange={(headers) => update({ headers })} />
    {:else if section === "auth"}
      <div class="fields">
        <label>
          Type
          <select value={form.auth.type} onchange={(e) => setAuth(e.currentTarget.value as Auth["type"])}>
            <option value="none">None</option>
            <option value="bearer">Bearer token</option>
            <option value="basic">Basic</option>
            <option value="apikey">API key header</option>
            <option value="oauth2">OAuth2 profile</option>
          </select>
        </label>
        {#if form.auth.type === "bearer"}
          <label>Token <input value={form.auth.token} spellcheck="false" placeholder={"{{token}}"} oninput={(e) => setAuthField("token", e.currentTarget.value)} /></label>
        {:else if form.auth.type === "basic"}
          <label>Username <input value={form.auth.username} spellcheck="false" oninput={(e) => setAuthField("username", e.currentTarget.value)} /></label>
          <label>Password <input value={form.auth.password} spellcheck="false" placeholder={"{{password}}"} oninput={(e) => setAuthField("password", e.currentTarget.value)} /></label>
        {:else if form.auth.type === "apikey"}
          <label>Header <input value={form.auth.header} spellcheck="false" oninput={(e) => setAuthField("header", e.currentTarget.value)} /></label>
          <label>Value <input value={form.auth.value} spellcheck="false" placeholder={"{{api_key}}"} oninput={(e) => setAuthField("value", e.currentTarget.value)} /></label>
        {:else if form.auth.type === "oauth2"}
          <label>Profile <input value={form.auth.profile} spellcheck="false" placeholder="api" oninput={(e) => setAuthField("profile", e.currentTarget.value)} /></label>
          <p class="hint">Settings come from environment variables named <code>{form.auth.profile || "api"}.client_id</code>, <code>{form.auth.profile || "api"}.token_url</code> and so on.</p>
        {/if}
        {#if form.auth.type !== "none" && form.auth.type !== "oauth2"}
          <p class="hint">Put secrets in the environment's secrets file and refer to them as <code>{"{{name}}"}</code>; they stay out of git and out of history.</p>
        {/if}
      </div>
    {:else if section === "body"}
      {#if isGql}
        <div class="split">
          <label class="area">Query<textarea value={form.body} spellcheck="false" oninput={(e) => update({ body: e.currentTarget.value })}></textarea></label>
          <label class="area">Variables (JSON)<textarea value={form.variables} spellcheck="false" placeholder={'{"id": 1}'} oninput={(e) => update({ variables: e.currentTarget.value })}></textarea></label>
        </div>
      {:else if isWs}
        <label class="area">
          Sent when the connection opens. Separate messages with a line containing only <code>===</code>.
          <textarea value={form.body} spellcheck="false" oninput={(e) => update({ body: e.currentTarget.value, body_mode: e.currentTarget.value ? "text" : "none" })}></textarea>
        </label>
      {:else}
        <div class="modes" role="radiogroup" aria-label="Body type">
          {#each [["none", "None"], ["json", "JSON"], ["text", "Text"], ["form", "Form"]] as [m, label] (m)}
            <button role="radio" aria-checked={form.body_mode === m} class:on={form.body_mode === m} onclick={() => setBodyMode(m as BodyMode)}>{label}</button>
          {/each}
          {#if form.body_mode === "json"}<button class="fmt" onclick={formatJson}>Format</button>{/if}
        </div>
        {#if form.body_mode === "json" || form.body_mode === "text"}
          <textarea class="body" value={form.body} spellcheck="false" oninput={(e) => update({ body: e.currentTarget.value })}></textarea>
        {:else if form.body_mode === "form"}
          <KeyValues rows={form.form} keyPlaceholder="field" onchange={(rows) => update({ form: rows })} />
        {:else}
          <p class="hint">This request has no body.</p>
        {/if}
      {/if}
    {:else if section === "settings"}
      <div class="fields">
        <label>
          Name
          <input value={form.name} spellcheck="false" placeholder="login" oninput={(e) => update({ name: e.currentTarget.value })} />
        </label>
        <p class="hint">Named requests can be used by later ones: <code>{`{{${form.name || "login"}.response.body.$.token}}`}</code>.</p>
        <label>
          Timeout
          <input value={form.timeout} spellcheck="false" placeholder="30s" oninput={(e) => update({ timeout: e.currentTarget.value })} />
        </label>
        <label class="check">
          <input type="checkbox" checked={!form.no_redirect} onchange={(e) => update({ no_redirect: !e.currentTarget.checked })} />
          Follow redirects
        </label>
        <label class="check">
          <input type="checkbox" checked={!form.insecure} onchange={(e) => update({ insecure: !e.currentTarget.checked })} />
          Check TLS certificates
        </label>
      </div>
    {/if}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .req {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }

  .line {
    display: flex;
    gap: 6px;
    padding: 10px 12px 8px;
  }

  .method,
  .kind {
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-weight: 800;
    color: var(--amber-ink);
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    background: var(--paper-raised);
    padding: 4px 8px;
  }

  .url {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    background: var(--paper-raised);
    padding: 5px 9px;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 13px;
  }

  .url:focus,
  .fields input:focus,
  textarea:focus,
  select:focus {
    outline: none;
    border-color: var(--amber);
  }

  .send {
    min-width: 84px;
  }

  .send:disabled {
    opacity: 0.6;
  }

  .sections {
    display: flex;
    gap: 2px;
    padding: 0 12px;
    border-bottom: 1px solid var(--rule-strong);
  }

  .sections button {
    border: none;
    background: transparent;
    padding: 5px 10px;
    color: var(--ink-soft);
    font-size: 13px;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }

  .sections button.on {
    color: var(--ink);
    border-bottom-color: var(--amber);
  }

  .n {
    margin-left: 5px;
    font-size: 11px;
    color: var(--ink-faint);
  }

  .section {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 8px 12px 12px;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 9px;
    max-width: 520px;
  }

  .fields label {
    display: grid;
    grid-template-columns: 90px 1fr;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: var(--ink-soft);
  }

  .fields label.check {
    display: flex;
    color: var(--ink);
  }

  .fields input:not([type="checkbox"]),
  .fields select {
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    background: var(--paper-raised);
    padding: 5px 8px;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    color: var(--ink);
  }

  .fields select {
    font-family: var(--font-ui);
    font-variation-settings: normal;
  }

  .modes {
    display: flex;
    gap: 2px;
    margin-bottom: 8px;
  }

  .modes button {
    border: 1px solid var(--rule-strong);
    background: transparent;
    padding: 3px 12px;
    font-size: 12.5px;
    color: var(--ink-soft);
  }

  .modes button:first-child {
    border-radius: 6px 0 0 6px;
  }

  .modes button:nth-child(4) {
    border-radius: 0 6px 6px 0;
  }

  .modes button.on {
    background: var(--amber-wash);
    color: var(--ink);
    border-color: var(--amber);
  }

  .modes .fmt {
    margin-left: auto;
    border-radius: 6px;
  }

  textarea {
    width: 100%;
    min-height: 140px;
    resize: vertical;
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    background: var(--paper-raised);
    padding: 8px 10px;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    line-height: 19px;
    color: var(--ink);
    tab-size: 2;
  }

  .area {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12.5px;
    color: var(--ink-soft);
    flex: 1;
  }

  .split {
    display: flex;
    gap: 10px;
  }

  .hint {
    color: var(--ink-faint);
    font-size: 12.5px;
    margin: 8px 0 0;
  }

  code {
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    background: var(--code-bg);
    padding: 0 3px;
    border-radius: 3px;
  }

  .error {
    margin: 0;
    padding: 6px 12px;
    color: var(--err);
    background: var(--err-wash);
    font-size: 13px;
  }
</style>
