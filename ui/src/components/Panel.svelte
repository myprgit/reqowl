<script lang="ts">
  import {
    type BlockInfo,
    type HistoryEntry,
    type RequestForm,
    type Shown,
    type Socket,
    formatSize,
    isJson,
    prettyBody,
    statusClass,
  } from "../lib/api";
  import Viewer from "./Viewer.svelte";
  import RequestEditor from "./RequestEditor.svelte";

  export type Tab = "edit" | "body" | "headers" | "request" | "history" | "messages";

  interface Props {
    shown: Shown | undefined;
    socket: Socket | undefined;
    history: HistoryEntry[];
    blockLabel: BlockInfo | undefined;
    onClose: () => void;
    onPin: () => void;
    onCurl: () => void;
    onPickHistory: (e: HistoryEntry) => void;
    onSendMessage: (text: string) => void;
    onDisconnect: () => void;
    onReconnect: () => void;
    tab: Tab;
    form: RequestForm | null;
    formError: string | null;
    running: boolean;
    formFocused: boolean;
    onFormChange: (f: RequestForm) => void;
    onSend: () => void;
  }
  let { tab = $bindable(), formFocused = $bindable(), ...p }: Props = $props();
  let message = $state("");
  let logEl: HTMLDivElement | undefined = $state();

  const isSocket = $derived(p.blockLabel?.kind === "websocket" || !!p.socket);
  const cls = $derived(statusClass(p.shown));
  const body = $derived(p.shown ? prettyBody(p.shown) : "");
  const json = $derived(p.shown ? isJson(p.shown.contentType, p.shown.body) : false);

  $effect(() => {
    if (isSocket && tab !== "messages" && tab !== "history" && tab !== "edit") tab = "messages";
    if (!isSocket && tab === "messages") tab = "body";
  });

  $effect(() => {
    void p.socket?.log.length;
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  });

  function send(e: Event) {
    e.preventDefault();
    if (!message.trim()) return;
    p.onSendMessage(message);
    message = "";
  }

  function time(ts: number) {
    return new Date(ts).toLocaleTimeString();
  }

  function copy(text: string) {
    navigator.clipboard.writeText(text);
  }
</script>

<section class="panel">
  <header>
    <div class="what">
      {#if p.blockLabel}
        <span class="method">{p.blockLabel.kind === "websocket" ? "WS" : p.blockLabel.method}</span>
        <span class="url" title={p.blockLabel.url ?? ""}>{p.blockLabel.name ? `@${p.blockLabel.name}` : p.blockLabel.url}</span>
      {/if}
      {#if p.shown && !isSocket}
        <span class="status {cls}">{p.shown.error ? (p.shown.status ?? "Failed") : `${p.shown.status} ${p.shown.reason}`}</span>
        {#if p.shown.elapsedMs != null}<span class="meta">{p.shown.elapsedMs} ms</span>{/if}
        {#if p.shown.size != null}<span class="meta">{formatSize(p.shown.size)}</span>{/if}
        {#if p.shown.env}<span class="meta">env {p.shown.env}</span>{/if}
        {#if p.shown.stale}<span class="meta">from history, {new Date(p.shown.ts).toLocaleString()}</span>{/if}
      {/if}
      {#if p.socket}
        <span class="status {p.socket.open ? 'ok' : 'none'}">{p.socket.open ? "Connected" : "Closed"}</span>
      {/if}
    </div>
    <nav class="tabs" aria-label="Response views">
      <button class:on={tab === "edit"} onclick={() => (tab = "edit")} title="Ctrl+E">Edit</button>
      {#if isSocket}
        <button class:on={tab === "messages"} onclick={() => (tab = "messages")}>Messages</button>
      {:else}
        <button class:on={tab === "body"} onclick={() => (tab = "body")}>Body</button>
        <button class:on={tab === "headers"} onclick={() => (tab = "headers")}>Headers</button>
        <button class:on={tab === "request"} onclick={() => (tab = "request")}>Request</button>
      {/if}
      <button class:on={tab === "history"} onclick={() => (tab = "history")}>History</button>
    </nav>
    <div class="actions">
      {#if !isSocket}
        <button class="quiet-btn" onclick={() => copy(body)} disabled={!body}>Copy body</button>
        <button class="quiet-btn" onclick={p.onCurl}>Copy as curl</button>
        <button class="quiet-btn" onclick={p.onPin} disabled={!p.shown || p.shown.historyId == null} title="Write this response into the page">Pin to page</button>
      {/if}
      <button class="quiet-btn" onclick={p.onClose} title="Ctrl+J">Close</button>
    </div>
  </header>

  <div class="content">
    {#if tab === "edit"}
      {#if p.form}
        <RequestEditor
          form={p.form}
          error={p.formError}
          running={p.running}
          bind:focused={formFocused}
          onchange={p.onFormChange}
          onsend={p.onSend}
        />
      {:else}
        <p class="hint">{p.formError ?? "This block can't be shown as a form."}</p>
      {/if}
    {:else if tab === "messages"}
      <div class="log" bind:this={logEl}>
        {#if !p.socket}
          <p class="hint">Run the block to connect.</p>
        {:else}
          {#each p.socket.log as m, i (i)}
            {@const ev = m.event}
            <div class="msg {ev.kind}">
              <span class="t">{time(m.ts)}</span>
              {#if ev.kind === "opened"}
                <span>Connected ({ev.status}) in {ev.elapsed_ms} ms</span>
              {:else if ev.kind === "sent"}
                <span class="dir">→</span><pre>{ev.text}</pre>
              {:else if ev.kind === "received"}
                <span class="dir">←</span><pre>{ev.text}</pre>
              {:else if ev.kind === "received_binary"}
                <span class="dir">←</span><span>{formatSize(ev.size)} of binary data</span>
              {:else if ev.kind === "closed"}
                <span>Closed{ev.code != null ? ` (${ev.code}${ev.reason ? `: ${ev.reason}` : ""})` : ""}</span>
              {:else if ev.kind === "error"}
                <span class="err">{ev.message}</span>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
      <form class="send" onsubmit={send}>
        <input bind:value={message} placeholder={p.socket?.open ? "Message to send" : "Not connected"} disabled={!p.socket?.open} />
        {#if p.socket?.open}
          <button class="ink-btn" disabled={!message.trim()}>Send</button>
          <button type="button" class="quiet-btn" onclick={p.onDisconnect}>Disconnect</button>
        {:else}
          <button type="button" class="ink-btn" onclick={p.onReconnect}>Connect</button>
        {/if}
      </form>
    {:else if tab === "history"}
      <div class="history">
        {#if p.history.length === 0}
          <p class="hint">No runs of this block yet.</p>
        {/if}
        {#each p.history as h (h.id)}
          {@const hc = h.error ? "err" : (h.status ?? 0) >= 400 ? "err" : (h.status ?? 0) >= 300 ? "warn" : "ok"}
          <button class="hrow" class:on={p.shown?.historyId === h.id} onclick={() => p.onPickHistory(h)}>
            <span class="status {hc}">{h.error ? "Failed" : h.status}</span>
            <span class="when">{new Date(h.ts).toLocaleString([], { dateStyle: "short", timeStyle: "medium" })}</span>
            <span class="meta">{h.env ?? "no env"}</span>
            <span class="meta">{h.elapsed_ms != null ? `${h.elapsed_ms} ms` : ""}</span>
            <span class="meta url">{h.url}</span>
          </button>
        {/each}
      </div>
    {:else if !p.shown}
      <p class="hint">Run the block to see its response here.</p>
    {:else if p.shown.error}
      <div class="error">
        <strong>The request didn't complete.</strong>
        <pre>{p.shown.error}</pre>
      </div>
    {:else if tab === "body"}
      {#if p.shown.image}
        <div class="img"><img src={p.shown.image} alt="Response" /></div>
      {:else if body}
        <Viewer doc={body} {json} />
      {:else}
        <p class="hint">The response has no body.</p>
      {/if}
      {#if p.shown.truncated}
        <p class="hint">This body was cut off{p.shown.stale ? " in history (2 MB kept)" : " at 64 MB"}.</p>
      {/if}
    {:else if tab === "headers"}
      <table>
        <tbody>
          {#each p.shown.headers as [k, v], i (i)}
            <tr><th>{k}</th><td>{v}</td></tr>
          {/each}
        </tbody>
      </table>
    {:else if tab === "request" && p.shown.request}
      {@const r = p.shown.request}
      <div class="req">
        <pre class="line">{r.method} {r.url}</pre>
        <table>
          <tbody>
            {#each r.headers as [k, v], i (i)}
              <tr><th>{k}</th><td>{v}</td></tr>
            {/each}
          </tbody>
        </table>
        {#if r.body}<pre class="rbody">{r.body}</pre>{/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .panel {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--paper-side);
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 5px 10px 5px 16px;
    border-bottom: 1px solid var(--rule-strong);
    flex-wrap: wrap;
  }

  .what {
    display: flex;
    gap: 10px;
    align-items: baseline;
    min-width: 0;
    flex: 1;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
  }

  .method {
    font-weight: 800;
    color: var(--accent-ink);
  }

  .url {
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 34ch;
    color: var(--ink-soft);
  }

  .status {
    font-weight: 700;
  }
  .status.ok {
    color: var(--ok);
  }
  .status.warn {
    color: var(--warn);
  }
  .status.err {
    color: var(--err);
  }
  .status.none {
    color: var(--ink-faint);
  }

  .meta {
    color: var(--ink-faint);
  }

  .tabs {
    display: flex;
    gap: 2px;
  }

  .tabs button {
    border: none;
    background: transparent;
    padding: 4px 10px;
    border-radius: 5px;
    color: var(--ink-soft);
    font-size: 13px;
  }

  .tabs button.on {
    background: var(--paper);
    color: var(--ink);
    box-shadow: inset 0 -2px 0 var(--accent);
  }

  .actions {
    display: flex;
    gap: 2px;
  }

  .actions button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .content {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--paper);
    display: flex;
    flex-direction: column;
  }

  .hint {
    color: var(--ink-faint);
    padding: 14px 18px;
    margin: 0;
  }

  .error {
    padding: 14px 18px;
    color: var(--err);
  }

  .error pre,
  .req pre,
  .msg pre {
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    white-space: pre-wrap;
    word-break: break-all;
    margin: 6px 0;
  }

  table {
    border-collapse: collapse;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
    margin: 8px 14px;
  }

  th {
    text-align: left;
    color: var(--ink-soft);
    font-weight: 600;
    padding: 2px 18px 2px 0;
    vertical-align: top;
    white-space: nowrap;
  }

  td {
    word-break: break-all;
    padding: 2px 0;
  }

  .req {
    padding: 6px 14px;
  }

  .req .line {
    font-weight: 700;
  }

  .rbody {
    border-top: 1px dashed var(--rule-strong);
    padding-top: 6px;
  }

  .img {
    padding: 14px;
  }

  .img img {
    max-width: 100%;
    border: 1px solid var(--rule);
  }

  .history {
    padding: 6px;
  }

  .hrow {
    display: grid;
    grid-template-columns: 56px 210px 80px 70px 1fr;
    gap: 10px;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 4px 10px;
    border-radius: 5px;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
  }

  .hrow:hover {
    background: var(--paper-raised);
  }

  .hrow.on {
    background: var(--accent-wash);
  }

  .hrow .url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .log {
    flex: 1;
    overflow: auto;
    padding: 6px 14px;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
    font-size: 12.5px;
  }

  .msg {
    display: flex;
    gap: 10px;
    align-items: baseline;
    border-bottom: 1px solid var(--rule);
    padding: 2px 0;
  }

  .msg pre {
    margin: 0;
  }

  .msg .t {
    color: var(--ink-faint);
    flex-shrink: 0;
  }

  .msg .dir {
    font-weight: 800;
  }

  .msg.sent .dir {
    color: var(--accent-ink);
  }

  .msg.received .dir {
    color: var(--ok);
  }

  .msg .err {
    color: var(--err);
  }

  .send {
    display: flex;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--rule-strong);
    background: var(--paper-side);
  }

  .send input {
    flex: 1;
    border: 1px solid var(--rule-strong);
    border-radius: 6px;
    background: var(--paper);
    padding: 5px 8px;
    font-family: var(--font-code);
    font-variation-settings: var(--code-axes);
  }
</style>
