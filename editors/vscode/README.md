# X Language — VSCode Extension

Language support for [xlang](https://github.com/Yueon-Labs/xlang) (`.x` files) in VSCode:
- **Live diagnostics** — type/parse errors underlined as you type
- **Hover** — function/struct signatures on hover
- **Go to definition** — jump to a function/struct's definition
- **Completion** — top-level function & struct names
- **Syntax highlighting** — keywords, types, strings, comments

It talks LSP over stdio to the `xlang-lsp` server (shipped with the compiler).

## Setup

### 1. Build the language server
From the xlang compiler repo:
```sh
cargo build --release      # produces target/release/xlang-lsp
```
Ensure `xlang-lsp` is on your `PATH`, **or** note its full path.

### 2. Install the extension
From this directory:
```sh
npm install
npx vsce package            # produces xlang-0.1.0.vsix
code --install-extension xlang-0.1.0.vsix
```

### 3. Point at the server (if not on PATH)
In VSCode settings (`settings.json`):
```json
{ "xlang.serverPath": "/absolute/path/to/xlang-lsp" }
```

## Use
Open any `.x` file. You should see syntax highlighting immediately, and live
errors / hover / go-to-definition once the server starts (it launches on first
`.x` file open).

Verify the server standalone:
```sh
echo 'Content-Length: 123' ...   # see xlang-lsp --help / src/bin/xlang-lsp.rs
```

## How it works
- `src/extension.js` — launches `xlang-lsp` via `vscode-languageclient` (stdio).
- `syntaxes/xlang.tmLanguage.json` — TextMate grammar for highlighting.
- `language-configuration.json` — `//` comments, bracket matching, auto-closing.

The server logic lives in the compiler (`src/lsp.rs`): `diagnostics`, `hover`,
`definition`, `completion_names` — all unit-tested there.
