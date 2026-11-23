# Writing a theme

A reqowl theme is a TOML file in the app's themes folder. Open the folder
from **Settings → Your own themes → Open themes folder**. Its location:

| System | Folder |
|---|---|
| Linux | `~/.config/dev.reqowl.app/themes/` |
| macOS | `~/Library/Application Support/dev.reqowl.app/themes/` |
| Windows | `%APPDATA%\dev.reqowl.app\themes\` |

The folder starts with a commented example, `_example.toml`. Files whose
names start with `_` are ignored, so copy it to a new name (for example
`forest.toml`), edit it, and press **Reload themes**. Your theme then shows up
next to the built-in ones.

```toml
name = "Forest"
dark = true          # used for "Match the system's light or dark mode"

[colors]
paper = "#16201b"
accent = "#9ad17c"
# ...

[fonts]              # optional, CSS font-family lists of installed fonts
note = "\"IBM Plex Sans\", sans-serif"
code = "\"JetBrains Mono\", monospace"
ui = "system-ui, sans-serif"
```

Every colour is optional. Missing ones come from the built-in Paper theme,
or from Ink when `dark = true`. Values are any CSS colour (`#rrggbb`,
`rgb(…)`, `oklch(…)`).

| Key | Used for |
|---|---|
| `paper` | Page background |
| `paper-side` | Sidebar and panels |
| `paper-raised` | Request blocks, inputs, menus |
| `rule` | Ruled lines on the page |
| `rule-strong` | Borders |
| `ink` | Text |
| `ink-soft` | Secondary text |
| `ink-faint` | Hints, timestamps, comments |
| `accent` | The margin line, focus rings, selected items |
| `accent-ink` | Accent used as text: `{{variables}}`, methods, directives |
| `accent-wash` | Selected rows, variable backgrounds |
| `ok`, `ok-wash` | 2xx responses |
| `warn` | 3xx responses |
| `err`, `err-wash` | Errors and 4xx/5xx responses |
| `select` | Text selection |
| `code-bg` | Inline `code` in notes |

Fonts set by a theme apply when **Settings → Text** is left on "Theme
default"; a font picked there wins over the theme's.

If a file has a TOML error or an unknown colour name, Settings lists it
under "Your own themes".
