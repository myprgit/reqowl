// Themes are sets of CSS variables. Built-in themes live here; anyone can add
// their own as TOML files in the app's themes folder (see docs/themes.md).

export const COLOR_KEYS = [
  "paper",
  "paper-side",
  "paper-raised",
  "rule",
  "rule-strong",
  "ink",
  "ink-soft",
  "ink-faint",
  "accent",
  "accent-ink",
  "accent-wash",
  "ok",
  "ok-wash",
  "warn",
  "err",
  "err-wash",
  "select",
  "code-bg",
] as const;

export type ColorKey = (typeof COLOR_KEYS)[number];

export interface Theme {
  id: string;
  name: string;
  dark: boolean;
  colors: Partial<Record<ColorKey, string>>;
  /** CSS font-family values; empty means the app default. */
  fonts?: { note?: string; code?: string; ui?: string };
  /** Where a custom theme came from, for the settings screen. */
  file?: string;
}

const paper: Theme = {
  id: "paper",
  name: "Paper",
  dark: false,
  colors: {
    paper: "#f5efe0",
    "paper-side": "#ece3cd",
    "paper-raised": "#fbf7ec",
    rule: "#e2d8c0",
    "rule-strong": "#cdbf9f",
    ink: "#1f2a44",
    "ink-soft": "#56607a",
    "ink-faint": "#8a8f9c",
    accent: "#e8a33d",
    "accent-ink": "#8a5a12",
    "accent-wash": "#f6e2b8",
    ok: "#2f6b45",
    "ok-wash": "#dcead9",
    warn: "#8a5a12",
    err: "#a63a28",
    "err-wash": "#f4dcd3",
    select: "#f3d9a3",
    "code-bg": "#efe6d1",
  },
};

export const BUILTIN_THEMES: Theme[] = [
  paper,
  {
    id: "ink",
    name: "Ink",
    dark: true,
    colors: {
      paper: "#1b2337",
      "paper-side": "#161d2e",
      "paper-raised": "#222b42",
      rule: "#273150",
      "rule-strong": "#34406a",
      ink: "#f0e8d6",
      "ink-soft": "#b5b0a3",
      "ink-faint": "#7f8296",
      accent: "#e8a33d",
      "accent-ink": "#f0b85e",
      "accent-wash": "#3a3424",
      ok: "#8cc79c",
      "ok-wash": "#1f3a31",
      warn: "#f0b85e",
      err: "#ef8f7a",
      "err-wash": "#43272a",
      select: "#4a4023",
      "code-bg": "#20293f",
    },
  },
  {
    id: "kraft",
    name: "Kraft",
    dark: false,
    colors: {
      paper: "#e6d6b8",
      "paper-side": "#dccaa7",
      "paper-raised": "#efe3cb",
      rule: "#d2bd96",
      "rule-strong": "#b99f72",
      ink: "#2a2118",
      "ink-soft": "#5b4a37",
      "ink-faint": "#8a7760",
      accent: "#2f6b5a",
      "accent-ink": "#245446",
      "accent-wash": "#cfd9c3",
      ok: "#2f6b45",
      "ok-wash": "#d3dcc0",
      warn: "#8a5a12",
      err: "#9c3523",
      "err-wash": "#e8c9b8",
      select: "#c9d6bd",
      "code-bg": "#e0cfae",
    },
  },
  {
    id: "graph",
    name: "Graph",
    dark: false,
    colors: {
      paper: "#f7f9fb",
      "paper-side": "#edf1f6",
      "paper-raised": "#ffffff",
      rule: "#dfe7f0",
      "rule-strong": "#c3cfdc",
      ink: "#18222e",
      "ink-soft": "#4a5868",
      "ink-faint": "#8593a3",
      accent: "#2f6fde",
      "accent-ink": "#1f56b8",
      "accent-wash": "#dde8fb",
      ok: "#1f7a4c",
      "ok-wash": "#dbf0e3",
      warn: "#9a6410",
      err: "#c0362c",
      "err-wash": "#f8dedb",
      select: "#cfe0fb",
      "code-bg": "#eef2f7",
    },
  },
  {
    id: "blueprint",
    name: "Blueprint",
    dark: true,
    colors: {
      paper: "#0f2d55",
      "paper-side": "#0b2446",
      "paper-raised": "#143663",
      rule: "#1a4274",
      "rule-strong": "#2b5a92",
      ink: "#e8f1ff",
      "ink-soft": "#a9c2e2",
      "ink-faint": "#6f90ba",
      accent: "#ffd34d",
      "accent-ink": "#ffd34d",
      "accent-wash": "#2a4a6a",
      ok: "#8fe0b0",
      "ok-wash": "#1b4a4a",
      warn: "#ffd34d",
      err: "#ff9b86",
      "err-wash": "#4a2f45",
      select: "#2d527f",
      "code-bg": "#123461",
    },
  },
  {
    id: "night",
    name: "Night",
    dark: true,
    colors: {
      paper: "#111215",
      "paper-side": "#0b0c0e",
      "paper-raised": "#18191d",
      rule: "#202329",
      "rule-strong": "#31353d",
      ink: "#f1f1ee",
      "ink-soft": "#b9bdc5",
      "ink-faint": "#7c818b",
      accent: "#7fd1b9",
      "accent-ink": "#7fd1b9",
      "accent-wash": "#1c302c",
      ok: "#7fd1b9",
      "ok-wash": "#16302a",
      warn: "#f0c36b",
      err: "#ff8a7a",
      "err-wash": "#3a2224",
      select: "#2a3d3a",
      "code-bg": "#1a1c21",
    },
  },
];

export interface FontPreset {
  id: string;
  label: string;
  family: string;
  /** font-variation-settings, for Recursive's axes. */
  axes: string;
}

const RECURSIVE = '"Recursive Variable", ui-monospace, monospace';

export const NOTE_FONTS: FontPreset[] = [
  { id: "recursive-casual", label: "Recursive Casual", family: RECURSIVE, axes: '"MONO" 0.5, "CASL" 1, "slnt" 0, "CRSV" 0.5' },
  { id: "recursive-mono", label: "Recursive Mono", family: RECURSIVE, axes: '"MONO" 1, "CASL" 0, "slnt" 0, "CRSV" 0' },
  { id: "recursive-sans", label: "Recursive Sans", family: RECURSIVE, axes: '"MONO" 0, "CASL" 0, "slnt" 0, "CRSV" 0' },
  { id: "atkinson", label: "Atkinson Hyperlegible", family: '"Atkinson Hyperlegible Next Variable", system-ui, sans-serif', axes: "normal" },
  { id: "system-sans", label: "System sans-serif", family: "system-ui, sans-serif", axes: "normal" },
  { id: "system-serif", label: "System serif", family: "Georgia, 'Times New Roman', serif", axes: "normal" },
  { id: "system-mono", label: "System monospace", family: "ui-monospace, 'DejaVu Sans Mono', Menlo, Consolas, monospace", axes: "normal" },
];

export const CODE_FONTS: FontPreset[] = [
  { id: "recursive-mono", label: "Recursive Mono", family: RECURSIVE, axes: '"MONO" 1, "CASL" 0, "slnt" 0, "CRSV" 0' },
  { id: "recursive-casual-mono", label: "Recursive Mono Casual", family: RECURSIVE, axes: '"MONO" 1, "CASL" 1, "slnt" 0, "CRSV" 0' },
  { id: "system-mono", label: "System monospace", family: "ui-monospace, 'DejaVu Sans Mono', Menlo, Consolas, monospace", axes: "normal" },
];

export interface Settings {
  /** A theme id, or "system" to follow the OS light/dark setting. */
  theme: string;
  lightTheme: string;
  darkTheme: string;
  /** A preset id, "theme" for the theme's choice, or "custom". */
  noteFont: string;
  noteFontCustom: string;
  codeFont: string;
  codeFontCustom: string;
  fontSize: number;
  ruled: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  lightTheme: "paper",
  darkTheme: "ink",
  noteFont: "theme",
  noteFontCustom: "",
  codeFont: "theme",
  codeFontCustom: "",
  fontSize: 15,
  ruled: true,
};

export function loadSettings(): Settings {
  try {
    const saved = JSON.parse(localStorage.getItem("reqowl.settings") ?? "{}");
    const legacy = localStorage.getItem("reqowl.theme");
    const s = { ...DEFAULT_SETTINGS, ...saved };
    if (!saved.theme && (legacy === "light" || legacy === "dark")) s.theme = legacy === "dark" ? "ink" : "paper";
    return s;
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(s: Settings) {
  localStorage.setItem("reqowl.settings", JSON.stringify(s));
}

export function resolveTheme(s: Settings, themes: Theme[], systemDark: boolean): Theme {
  const byId = (id: string) => themes.find((t) => t.id === id);
  const id = s.theme === "system" ? (systemDark ? s.darkTheme : s.lightTheme) : s.theme;
  return byId(id) ?? (systemDark ? byId("ink")! : paper);
}

function fontFor(choice: string, custom: string, presets: FontPreset[], themeFamily: string | undefined, fallback: FontPreset) {
  if (choice === "custom" && custom.trim()) return { family: `${custom.trim()}, ${fallback.family}`, axes: "normal" };
  const preset = presets.find((p) => p.id === choice);
  if (preset) return preset;
  if (themeFamily) return { family: themeFamily, axes: "normal" };
  return fallback;
}

/** Writes a theme and the font settings into CSS variables on the page. */
export function applyAppearance(s: Settings, theme: Theme) {
  const root = document.documentElement;
  const base = theme.dark ? BUILTIN_THEMES[1].colors : paper.colors;
  for (const key of COLOR_KEYS) {
    root.style.setProperty(`--${key}`, theme.colors[key] ?? base[key] ?? "");
  }
  root.dataset.theme = theme.dark ? "dark" : "light";

  const note = fontFor(s.noteFont, s.noteFontCustom, NOTE_FONTS, theme.fonts?.note, NOTE_FONTS[0]);
  const code = fontFor(s.codeFont, s.codeFontCustom, CODE_FONTS, theme.fonts?.code, CODE_FONTS[0]);
  root.style.setProperty("--font-note", note.family);
  root.style.setProperty("--note-axes", note.axes);
  root.style.setProperty("--font-code", code.family);
  root.style.setProperty("--code-axes", code.axes);
  if (theme.fonts?.ui) root.style.setProperty("--font-ui", theme.fonts.ui);
  else root.style.removeProperty("--font-ui");

  const size = Math.min(22, Math.max(11, s.fontSize));
  root.style.setProperty("--note-size", `${size}px`);
  root.style.setProperty("--code-size", `${Math.round(size * 0.9 * 2) / 2}px`);
  root.style.setProperty("--line", `${Math.round(size * 1.73)}px`);
  root.style.setProperty("--ruled", s.ruled ? "var(--rule)" : "transparent");
}

/** Custom theme as read by the backend from a TOML file. */
export interface CustomThemeFile {
  file: string;
  theme: { name?: string; dark?: boolean; colors?: Record<string, string>; fonts?: Record<string, string> } | null;
  error: string | null;
}

export function customThemes(files: CustomThemeFile[]): { themes: Theme[]; errors: string[] } {
  const themes: Theme[] = [];
  const errors: string[] = [];
  for (const f of files) {
    if (f.error || !f.theme) {
      errors.push(`${f.file}: ${f.error ?? "empty"}`);
      continue;
    }
    const unknown = Object.keys(f.theme.colors ?? {}).filter((k) => !(COLOR_KEYS as readonly string[]).includes(k));
    if (unknown.length) errors.push(`${f.file}: unknown colour${unknown.length > 1 ? "s" : ""} ${unknown.join(", ")}`);
    themes.push({
      id: `custom:${f.file}`,
      name: f.theme.name || f.file.replace(/\.toml$/, ""),
      dark: !!f.theme.dark,
      colors: f.theme.colors ?? {},
      fonts: f.theme.fonts,
      file: f.file,
    });
  }
  return { themes, errors };
}
