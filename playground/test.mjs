// Smoke test for the built module: `node playground/test.mjs`.
import { readFile } from "node:fs/promises";
import { wrap } from "./web/dog4-wasm.js";

const bytes = await readFile(new URL("./web/dog4.wasm", import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, {});
console.log("imports needed:", WebAssembly.Module.imports(await WebAssembly.compile(bytes)).length);
const dog4 = wrap(instance);
const checks = [
  ["put { put 1; put 2 }", (r) => r.ok && r.output === "3"],
  ["println hi; pln [a (null)]", (r) => r.ok && r.printed === "hi\n" && r.output === "[a (null)]\n"],
  ["pln ok;\npln $nope", (r) => !r.ok && r.line === 2 && r.kind === "name"],
  ["while true {}", (r) => !r.ok && r.kind === "limit"],
  ['use mod greet; hi Bob', (r) => r.ok && r.output === "Hello Bob\n"],
  ["put { random 10 }", (r) => r.ok && Number(r.output) < 10],
];
let failed = 0;
for (const [source, ok] of checks) {
  const result = dog4.run(source, 7);
  const pass = ok(result);
  if (!pass) failed++;
  console.log(pass ? "ok  " : "FAIL", JSON.stringify(source), "->", JSON.stringify(result).slice(0, 120));
}
console.log(dog4.disassemble("put 1").output.split("\n")[0]);
const big = dog4.run("s = for i in (0..200000) { put [$i] }; put { len $s }");
console.log("200k loop:", big.output);
process.exit(failed ? 1 : 0);
