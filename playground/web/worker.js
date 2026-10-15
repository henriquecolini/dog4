// Runs dog4 off the main thread, so a long script never freezes the page and
// the page can stop it by terminating the worker.

import { load } from "./dog4-wasm.js";

const ready = load(new URL("dog4.wasm", import.meta.url));

self.onmessage = async ({ data }) => {
  const { id, source, mode } = data;
  try {
    const dog4 = await ready;
    const started = performance.now();
    const result = mode === "disasm" ? dog4.disassemble(source) : dog4.run(source);
    self.postMessage({ id, result, ms: performance.now() - started });
  } catch (error) {
    self.postMessage({
      id,
      result: { ok: false, printed: "", error: `internal error: ${error.message}`, kind: "internal" },
      ms: 0,
      crashed: true,
    });
  }
};

ready.then(
  () => self.postMessage({ ready: true }),
  (error) => self.postMessage({ ready: false, error: String(error) }),
);
