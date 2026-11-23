# Editors: `smysl lsp-server`

RFC SMYSL-6 §6. The server is the `smysl` binary itself, so an editor needs only `smysl` on
`PATH`. Each recipe below declares two languages: `smysl` for `.smy` and `.smysl` documents, and
`smysl-query` for `.sq` queries. Both are served by one command:

```sh
smysl lsp-server            # stdio; the editor's root folder is the workspace
smysl lsp-server --library ~/corpus   # a library, read without opening it
```

These recipes follow each editor's documented configuration. They are not exercised in CI:
the server is tested through `tests/cmd_lsp_server.rs`, which speaks the protocol to the binary
as an editor does.

## Helix

`~/.config/helix/languages.toml`:

```toml
[language-server.smysl]
command = "smysl"
args = ["lsp-server"]

[[language]]
name = "smysl"
scope = "source.smysl"
file-types = ["smy", "smysl"]
roots = ["LIBRARY"]
comment-token = "#"
language-servers = ["smysl"]

[[language]]
name = "smysl-query"
scope = "source.sq"
file-types = ["sq"]
comment-token = "#"
language-servers = ["smysl"]
```

## Neovim (0.11 or later)

```lua
vim.filetype.add({ extension = { smy = "smysl", smysl = "smysl", sq = "smysl-query" } })

vim.lsp.config("smysl", {
  cmd = { "smysl", "lsp-server" },
  filetypes = { "smysl", "smysl-query" },
  root_markers = { "LIBRARY", ".git" },
})
vim.lsp.enable("smysl")
```

## VS Code

VS Code starts a language server from an extension. A minimal one is two files.

`package.json`:

```json
{
  "name": "smysl",
  "version": "0.0.1",
  "engines": { "vscode": "^1.90.0" },
  "main": "extension.js",
  "activationEvents": ["onLanguage:smysl", "onLanguage:smysl-query"],
  "contributes": {
    "languages": [
      { "id": "smysl", "extensions": [".smy", ".smysl"] },
      { "id": "smysl-query", "extensions": [".sq"] }
    ]
  },
  "dependencies": { "vscode-languageclient": "^9.0.1" }
}
```

`extension.js`:

```js
const { LanguageClient } = require("vscode-languageclient/node");

let client;
exports.activate = () => {
  client = new LanguageClient(
    "smysl",
    "smysl",
    { command: "smysl", args: ["lsp-server"] },
    { documentSelector: [{ language: "smysl" }, { language: "smysl-query" }] },
  );
  return client.start();
};
exports.deactivate = () => client && client.stop();
```

## Zed

Zed adds a language through an extension. Its `extension.toml` names the server for both
languages:

```toml
id = "smysl"
name = "smysl"
version = "0.0.1"
schema_version = 1

[language_servers.smysl]
name = "smysl"
languages = ["smysl", "smysl-query"]
```

Each language gets `languages/<name>/config.toml` with its `path_suffixes` (`["smy", "smysl"]`,
`["sq"]`), and the extension's `language_server_command` returns `smysl` with the argument
`lsp-server`.

## What it reads

- **The workspace** is `--library`, or else the editor's root folder (`rootUri`, then the first
  workspace folder). It is read once, when the editor starts the server.
- **A library** is read by `Library::read_only`: the marker and each shard's log, and nothing
  else. No migration, no `enforce_z`, no index and no lock, so the library is byte-identical
  afterwards. A shard migrated to redb as primary is left unread, and the log says so.
- **A folder** contributes its `.smy` and `.smysl` documents, to depth 8, at most 2,000 of them,
  each at most 4 MiB. Hidden directories, `target` and `node_modules` are skipped.
- **Labels are a document's own.** A reference to a unit in another document or the library is
  its `b3:` uid, which then resolves in `check`, hover, definition and references.
