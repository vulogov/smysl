# Editors: `smysl lsp-server`

RFC SMYSL-6 §6. The server is the `smysl` binary itself, so an editor needs only `smysl` on
`PATH`. Each recipe below declares two languages: `smysl` for `.smy` and `.smysl` documents, and
`smysl-query` for `.sq` queries. Both are served by one command:

```sh
smysl lsp-server            # stdio; the editor's root folder is the workspace
smysl lsp-server --library ~/corpus   # a library, read without opening it
```

The server is tested in CI through `tests/cmd_lsp_server.rs`, which speaks the protocol to the
binary as an editor does. The Neovim recipe was also run in Neovim 0.12.6 itself, headless, on
macOS (below). The Helix, VS Code and Zed recipes follow each editor's documented configuration
and have not been run in the editor.

**On macOS, a marker named `LIBRARY` is not enough.** The disk ignores case, so a root marker
`LIBRARY` also matches `/Library` and `~/Library`, and the editor takes `/` or the home folder for
the project. The Neovim recipe looks for a file named exactly `LIBRARY` for that reason.

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
comment-token = "#"
language-servers = ["smysl"]

[[language]]
name = "smysl-query"
scope = "source.sq"
file-types = ["sq"]
comment-token = "#"
language-servers = ["smysl"]
```

Helix's root is its workspace: the folder holding `.git` or `.helix`, else the folder it was
started in. To serve a library, start Helix in it, or pass `--library` in `args`. A `roots`
entry of `LIBRARY` is left out: on macOS it matches `~/Library` (above).

## Neovim (0.11 or later)

```lua
vim.filetype.add({ extension = { smy = "smysl", smysl = "smysl", sq = "smysl-query" } })

vim.lsp.config("smysl", {
  cmd = { "smysl", "lsp-server" },
  filetypes = { "smysl", "smysl-query" },
  -- A library is the folder holding a file named exactly LIBRARY; else the git root.
  root_dir = function(bufnr, on_dir)
    local library = vim.fs.root(bufnr, function(name, path)
      return name == "LIBRARY" and vim.fn.filereadable(path .. "/" .. name) == 1
    end)
    on_dir(library or vim.fs.root(bufnr, ".git") or vim.fn.getcwd())
  end,
})
vim.lsp.enable("smysl")
```

Run in Neovim 0.12.6 on macOS: the client attaches and agrees on UTF-8; it receives the
diagnostics, hover, definition and references (also into another document of the folder by
`b3:` uid), the outline, completion, the label quick fix applied through
`vim.lsp.util.apply_workspace_edit`, and `vim.lsp.buf.format`, which gives what `smysl fmt` gives
and, on a document with errors, shows the refusal as a message.

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
  each at most 4 MiB, looking at no more than 20,000 entries in all. Hidden directories,
  `target`, `node_modules` and links to directories are skipped. The filesystem root, which an
  editor sends when it finds no project, is not read.
- **Labels are a document's own.** A reference to a unit in another document or the library is
  its `b3:` uid, which then resolves in `check`, hover, definition and references.
