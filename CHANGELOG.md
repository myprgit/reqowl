# Changelog

## 0.1.0 (unreleased)

First usable version.

- Desktop app for Linux, macOS and Windows: page tree, Markdown editor with
  runnable `http` blocks, responses under each block, response panel with
  body, headers, request and history, light and dark themes.
- Request blocks in VS Code REST Client / JetBrains HTTP Client syntax, with
  `@name`, `@auth` (bearer, basic, API key, OAuth2), `@timeout`,
  `@no-redirect` and `@insecure`.
- Environments in `env/*.toml`, secrets in git-ignored `env/*.local.toml`,
  masked in history, CLI output and pinned responses.
- Chained requests: `{{name.response.body.$.path}}`; running a block runs the
  named blocks it depends on.
- GraphQL and WebSocket blocks; OAuth2 client credentials, authorization code
  with PKCE, and password grants.
- Import from Postman (collections and environments), Insomnia (v4 JSON, v5
  YAML) and curl; paste curl into a page; copy a block as curl.
- `reqowl` CLI to run pages from scripts and CI.
- Form editor for requests (params, headers, auth, body, settings), kept in
  sync with the block text both ways; "+ Request" adds a block from the form.
- Foldable responses, one by one or for the whole page.
- Settings: six built-in themes, follow-system light/dark, note and code
  fonts, text size, ruled lines on/off, and custom themes from TOML files.
