import "@fontsource-variable/recursive/full.css";
import "@fontsource-variable/atkinson-hyperlegible-next";
import "./theme.css";
import { mount } from "svelte";
import { invoke } from "@tauri-apps/api/core";

const report = (level: string, message: string) => invoke("log_ui", { level, message }).catch(() => {});
window.addEventListener("error", (e) => report("error", `${e.message} at ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report("error", `unhandled: ${e.reason?.stack ?? e.reason}`));
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });
