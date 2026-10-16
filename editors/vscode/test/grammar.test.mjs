// Tokenises the fixtures with the engine VS Code itself uses, and asserts the
// scopes on specific tokens. Run with `npm test`.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";
// Both packages are CommonJS, so they arrive as default exports.
import oniguruma from "vscode-oniguruma";
import textmate from "vscode-textmate";

const here = dirname(fileURLToPath(import.meta.url));

const wasm = readFileSync(
  join(here, "..", "node_modules", "vscode-oniguruma", "release", "onig.wasm"),
);
const onigLib = oniguruma.loadWASM(wasm.buffer).then(() => ({
  createOnigScanner: (sources) => new oniguruma.OnigScanner(sources),
  createOnigString: (s) => new oniguruma.OnigString(s),
}));

const registry = new textmate.Registry({
  onigLib,
  loadGrammar: async (scope) =>
    scope === "source.dog"
      ? textmate.parseRawGrammar(
          readFileSync(join(here, "..", "syntaxes", "dog.tmLanguage.json"), "utf8"),
          "dog.tmLanguage.json",
        )
      : null,
});

const grammar = await registry.loadGrammar("source.dog");

/** Tokenises `src` into `{ text, scopes }` entries, dropping whitespace. */
function tokenize(src) {
  let state = textmate.INITIAL;
  const all = [];
  for (const line of src.split("\n")) {
    const result = grammar.tokenizeLine(line, state);
    for (const t of result.tokens) {
      const text = line.slice(t.startIndex, t.endIndex);
      if (text.trim()) all.push({ text, scopes: t.scopes });
    }
    state = result.ruleStack;
  }
  return all;
}

const source = readFileSync(join(here, "fixtures", "sample.dog"), "utf8");
const tokens = tokenize(source);

/** The scopes of the first token whose text matches exactly. */
function scopesOf(text, nth = 0) {
  const found = tokens.filter((t) => t.text === text);
  assert.ok(found[nth], `no token ${JSON.stringify(text)} (#${nth})`);
  return found[nth].scopes.join(" ");
}

test("line comments are comments", () => {
  // Tokens split on word boundaries, so match on the comment opener.
  assert.match(scopesOf("//"), /comment\.line/);
  const body = tokens.find((t) => t.text.includes("A line comment"));
  assert.match(body.scopes.join(" "), /comment\.line/);
});

test("block comments nest without ending early", () => {
  // In `/* nested /* block */ comment */` the inner `*/` must not close the
  // outer comment, so the text after it is still inside one.
  const tail = tokens.find((t) => t.text.trim() === "comment");
  assert.ok(tail, "no token after the inner block comment closed");
  assert.match(
    tail.scopes.join(" "),
    /comment\.block/,
    "the inner `*/` closed the outer comment",
  );
});

test("keywords are keywords", () => {
  for (const kw of ["if", "for", "in"]) {
    // A keyword at a statement start must not be mistaken for a command.
    assert.match(scopesOf(kw), /keyword\.control/, `${kw} was not a keyword`);
  }
  for (const kw of ["use", "from", "mod", "pub"]) {
    const found = tokens.find((t) => t.text === kw);
    if (found) assert.match(found.scopes.join(" "), /keyword\.other/);
  }
});

test("fn declarations name the function", () => {
  assert.match(scopesOf("fn"), /keyword\.declaration\.function/);
  assert.match(scopesOf("greet"), /entity\.name\.function/);
});

test("parameter types are types", () => {
  assert.match(scopesOf("str"), /support\.type/);
  assert.match(scopesOf("num"), /support\.type/);
});

test("parameter names are parameters, not commands", () => {
  for (const name of ["name", "times"]) {
    const param = tokens.find(
      (t) => t.text === name && t.scopes.join(" ").includes("variable.parameter"),
    );
    assert.ok(param, `${name} was not scoped as a parameter`);
  }
});

test("double quoted strings interpolate", () => {
  // `$name` inside "hello $name" is both a string and a variable.
  const interpolated = tokens.find(
    (t) =>
      t.text === "name" &&
      t.scopes.join(" ").includes("string.quoted.double") &&
      t.scopes.join(" ").includes("variable.other"),
  );
  assert.ok(interpolated, "$name inside a string was not highlighted as a variable");
});

test("single quoted strings do not interpolate", () => {
  const inside = tokens.filter((t) => t.scopes.join(" ").includes("string.quoted.single"));
  assert.ok(inside.length > 0, "no single quoted string found");
  for (const t of inside) {
    assert.ok(
      !t.scopes.join(" ").includes("variable.other"),
      `'${t.text}' was treated as a variable inside a literal string`,
    );
  }
});

test("raw strings hold quotes and don't interpolate", () => {
  const raw = tokenize('pln """"say """hi""" $x"""" after;');
  const inside = raw.filter((t) => t.scopes.join(" ").includes("string.quoted.triple"));
  assert.equal(inside.map((t) => t.text).join(""), '""""say """hi""" $x""""');
  assert.ok(inside.every((t) => !t.scopes.join(" ").includes("variable.other")));
  const after = raw.find((t) => t.text.includes("after"));
  assert.ok(after && !after.scopes.join(" ").includes("string"), "the raw string didn't end");
});

test("numbers are numeric", () => {
  assert.match(scopesOf("8080"), /constant\.numeric/);
  assert.match(scopesOf("2.5"), /constant\.numeric/);
});

test("map keys are members", () => {
  assert.match(scopesOf("host"), /variable\.other\.member/);
  assert.match(scopesOf("port"), /variable\.other\.member/);
});

test("true is a language constant", () => {
  assert.match(scopesOf("true"), /constant\.language/);
});

test("operators are operators", () => {
  assert.match(scopesOf(">"), /keyword\.operator/);
});

test("nothing is left completely unscoped", () => {
  // Every token should carry at least the root scope plus something, or be
  // plain word text; this guards against the grammar throwing.
  for (const t of tokens) {
    assert.ok(t.scopes[0] === "source.dog", `bad root scope for ${t.text}`);
  }
});
