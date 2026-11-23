import "@fontsource-variable/recursive/full.css";
import "@fontsource-variable/atkinson-hyperlegible-next";
import "./theme.css";
import { mount } from "svelte";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });
