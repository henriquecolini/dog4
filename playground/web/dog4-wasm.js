// Loads dog4's WebAssembly module and runs code with it.
// Works in a browser (worker or page) and in Node.

/** Wraps an instantiated dog4 module. */
export function wrap(instance) {
  const wasm = instance.exports;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  function call(source, mode, seed) {
    const bytes = encoder.encode(source);
    const ptr = wasm.dog4_alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    const out = wasm.dog4_run(ptr, bytes.length, seed >>> 0, mode);
    const len = wasm.dog4_result_len();
    const json = decoder.decode(new Uint8Array(wasm.memory.buffer, out, len));
    wasm.dog4_free(ptr, bytes.length);
    return JSON.parse(json);
  }
  return {
    /** Runs a program: `{ ok, printed, output }` or `{ ok: false, printed, error, kind, line, col }`. */
    run: (source, seed = Math.floor(Math.random() * 2 ** 32)) => call(source, 0, seed),
    /** Compiles a program and returns its bytecode listing as `output`. */
    disassemble: (source) => call(source, 1, 0),
  };
}

/** Fetches and instantiates `dog4.wasm` from `url`. */
export async function load(url) {
  const response = await fetch(url);
  const { instance } = WebAssembly.instantiateStreaming
    ? await WebAssembly.instantiateStreaming(response, {})
    : await WebAssembly.instantiate(await response.arrayBuffer(), {});
  return wrap(instance);
}
