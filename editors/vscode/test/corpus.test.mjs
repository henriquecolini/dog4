// Tokenises every .dog file in the repository. This catches runaway begin/end
// rules, which are the usual way a TextMate grammar breaks: one bad string or
// comment pattern colours the rest of the file.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";
import oniguruma from "vscode-oniguruma";
import textmate from "vscode-textmate";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", "..");

const wasm = readFileSync(
  join(here, "..", "node_modules", "vscode-oniguruma", "release", "onig.wasm"),
);
const onigLib = oniguruma.loadWASM(wasm.buffer).then(() => ({
  createOnigScanner: (s) => new oniguruma.OnigScanner(s),
  createOnigString: (s) => new oniguruma.OnigString(s),
}));
const registry = new textmate.Registry({
  onigLib,
  loadGrammar: async () =>
    textmate.parseRawGrammar(
      readFileSync(join(here, "..", "syntaxes", "dog.tmLanguage.json"), "utf8"),
      "dog.tmLanguage.json",
    ),
});
const grammar = await registry.loadGrammar("source.dog");

/** Fixtures that are deliberately malformed, so they really do not terminate. */
const EXPECTED_UNTERMINATED = new Set([
  "tests/scripts/errors/syntax_unclosed_comment.dog",
  "tests/scripts/errors/syntax_unclosed_string.dog",
  "tests/scripts/words/raw_string_unclosed.dog",
]);

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry === ".git" || entry === "target") continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (entry.endsWith(".dog")) out.push(path);
  }
  return out;
}

const files = walk(root);

test("the corpus is not empty", () => {
  assert.ok(files.length > 50, `only found ${files.length} .dog files`);
});

test("every .dog file tokenises without running away", () => {
  const problems = [];
  for (const file of files) {
    const name = relative(root, file).replaceAll("\\", "/");
    let state = textmate.INITIAL;
    for (const line of readFileSync(file, "utf8").split("\n")) {
      const result = grammar.tokenizeLine(line, state);
      if (result.stoppedEarly) {
        problems.push(`${name}: tokeniser stopped early`);
        break;
      }
      state = result.ruleStack;
    }
    // Depth above the root means the file ended inside a string or comment.
    const unterminated = (state?.depth ?? 0) > 1;
    if (unterminated !== EXPECTED_UNTERMINATED.has(name)) {
      problems.push(
        unterminated
          ? `${name}: ended inside a string or comment`
          : `${name}: expected to be unterminated but was not`,
      );
    }
  }
  assert.deepEqual(problems, []);
});
