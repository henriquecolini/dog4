# dog4 for VS Code

Syntax highlighting for [dog4](https://github.com/henriquecolini/dog4) `.dog` files.

The grammar follows the lexical rules in `docs/language.md` and mirrors the
playground highlighter in `playground/web/highlight.js`: command words at
statement starts, `$` paths and interpolation, map keys, parameter types,
nestable block comments, and bare numeric literals.

## Installing locally

```sh
cd editors/vscode
npm install -g @vscode/vsce      # once
vsce package
code --install-extension dog4-0.1.0.vsix
```

Or, for development, symlink this directory into `~/.vscode/extensions/`.

## Testing the grammar

```sh
cd editors/vscode
npm install
npm test
```

`test/grammar.test.mjs` tokenises the fixtures in `test/fixtures/` with the
same engine VS Code uses (`vscode-textmate` + `vscode-oniguruma`) and asserts
the scopes assigned to specific tokens.
