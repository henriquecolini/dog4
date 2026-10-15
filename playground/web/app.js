// The dog4 playground: a highlighted editor, a worker running dog4 compiled to
// WebAssembly, examples, and shareable links.

import { highlight } from "./highlight.js";
import { EXAMPLES } from "./examples.js";

const $ = (id) => document.getElementById(id);
const code = $("code");
const painted = $("painted");
const paint = $("paint");
const gutter = $("gutter");
const output = $("output");
const status = $("status");
const runButton = $("run");
const stopButton = $("stop");
const bytecodeButton = $("bytecode");
const examples = $("examples");
const cursor = $("cursor");
const toast = $("toast");

const STORAGE_KEY = "dog4-playground-code";
const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
runButton.querySelector("kbd").textContent = isMac ? "⌘↵" : "Ctrl↵";
code.setAttribute("wrap", "off");

// ----- editor ---------------------------------------------------------------

let errorLine = null;

function render() {
  const text = code.value;
  const band = errorLine
    ? `<span class="error-band" style="top: ${(errorLine - 1) * lineHeight()}px"></span>`
    : "";
  painted.innerHTML = band + highlight(text) + "\n";
  const lines = text.split("\n").length;
  let numbers = "";
  for (let i = 1; i <= lines; i++) {
    numbers += i === errorLine ? `<div class="error">${i}</div>` : `<div>${i}</div>`;
  }
  gutter.innerHTML = numbers;
  syncScroll();
}

function lineHeight() {
  return parseFloat(getComputedStyle(code).lineHeight) || 22;
}

function syncScroll() {
  paint.scrollTop = code.scrollTop;
  paint.scrollLeft = code.scrollLeft;
  gutter.scrollTop = code.scrollTop;
}

function updateCursor() {
  const before = code.value.slice(0, code.selectionStart);
  const line = before.split("\n").length;
  const col = before.length - before.lastIndexOf("\n");
  cursor.textContent = `Ln ${line}, Col ${col}`;
}

/** Inserts text at the selection, keeping the browser's undo history. */
function insert(text) {
  code.focus();
  if (!document.execCommand("insertText", false, text)) {
    code.setRangeText(text, code.selectionStart, code.selectionEnd, "end");
    onEdit();
  }
}

function setCode(text) {
  code.value = text;
  code.selectionStart = code.selectionEnd = 0;
  code.scrollTop = 0;
  errorLine = null;
  onEdit();
}

let saveTimer = null;
function onEdit() {
  errorLine = null;
  render();
  updateCursor();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      localStorage.setItem(STORAGE_KEY, code.value);
    } catch {
      // Storage may be unavailable (private windows); the editor still works.
    }
  }, 300);
}

function currentLine() {
  const start = code.value.lastIndexOf("\n", code.selectionStart - 1) + 1;
  return { start, text: code.value.slice(start, code.selectionStart) };
}

code.addEventListener("input", onEdit);
code.addEventListener("scroll", syncScroll);
code.addEventListener("keyup", updateCursor);
code.addEventListener("click", updateCursor);
code.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
    event.preventDefault();
    run();
    return;
  }
  if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    const { selectionStart: start, selectionEnd: end, value } = code;
    if (start === end && !event.shiftKey) {
      insert("    ");
      return;
    }
    // Indent or outdent every selected line.
    const lineStart = value.lastIndexOf("\n", start - 1) + 1;
    const block = value.slice(lineStart, end);
    const changed = block
      .split("\n")
      .map((line) => (event.shiftKey ? line.replace(/^ {1,4}/, "") : "    " + line))
      .join("\n");
    code.setSelectionRange(lineStart, end);
    insert(changed);
    code.setSelectionRange(lineStart, lineStart + changed.length);
    return;
  }
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    const { text } = currentLine();
    let indent = /^\s*/.exec(text)[0];
    if (/[{[(]\s*$/.test(text)) indent += "    ";
    insert("\n" + indent);
    return;
  }
  if (event.key === "}" && code.selectionStart === code.selectionEnd) {
    const { start, text } = currentLine();
    if (/^\s+$/.test(text)) {
      event.preventDefault();
      code.setSelectionRange(start, code.selectionStart);
      insert(text.replace(/ {1,4}$/, "") + "}");
    }
  }
});

// ----- running ----------------------------------------------------------------

let worker = null;
let ready = false;
let nextId = 0;
let pending = null;
let stopTimer = null;
let ranOnce = false;

function startWorker() {
  ready = false;
  worker = new Worker(new URL("worker.js", import.meta.url), { type: "module" });
  worker.onmessage = ({ data }) => {
    if ("ready" in data) {
      ready = data.ready;
      status.textContent = ready ? "Ready" : `Couldn't load dog4: ${data.error}`;
      runButton.disabled = !ready;
      if (ready && !ranOnce) {
        ranOnce = true;
        run();
      }
      return;
    }
    if (!pending || data.id !== pending.id) return;
    finish(data);
  };
  worker.onerror = (event) => {
    status.textContent = `The runner crashed: ${event.message}`;
    finish(null);
    restartWorker();
  };
}

function restartWorker() {
  worker?.terminate();
  startWorker();
}

function run() {
  if (!ready || pending) return;
  const mode = bytecodeButton.getAttribute("aria-pressed") === "true" ? "disasm" : "run";
  pending = { id: ++nextId, mode };
  runButton.disabled = true;
  status.textContent = mode === "disasm" ? "Compiling…" : "Running…";
  stopTimer = setTimeout(() => {
    stopButton.hidden = false;
  }, 300);
  worker.postMessage({ id: pending.id, source: code.value, mode });
}

function finish(data) {
  clearTimeout(stopTimer);
  stopButton.hidden = true;
  runButton.disabled = !ready;
  const mode = pending?.mode;
  pending = null;
  if (!data) return;
  if (data.crashed) restartWorker();
  showResult(data.result, mode, data.ms);
}

stopButton.addEventListener("click", () => {
  if (!pending) return;
  pending = null;
  clearTimeout(stopTimer);
  stopButton.hidden = true;
  status.textContent = "Stopped";
  restartWorker();
});

const escapeHtml = (s) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

function showResult(result, mode, ms) {
  output.classList.toggle("bytecode", mode === "disasm");
  $("output-title").textContent = mode === "disasm" ? "Bytecode" : "Output";
  let html = "";
  if (result.printed) html += `<span class="printed">${escapeHtml(result.printed)}</span>`;
  if (result.ok) {
    if (result.output) {
      if (result.printed && !result.printed.endsWith("\n")) html += "\n";
      html += `<span class="result">${escapeHtml(result.output)}</span>`;
    }
    if (!html) html = '<span class="empty">The program ran and output nothing.</span>';
    status.textContent = `Finished in ${formatMs(ms)}`;
    errorLine = null;
  } else {
    if (result.printed && !result.printed.endsWith("\n")) html += "\n";
    html += `<span class="error">${escapeHtml(result.error)}</span>`;
    if (result.line) {
      html += `\n\n<button class="jump" type="button" data-line="${result.line}" data-col="${result.col}">Go to line ${result.line}</button>`;
    }
    status.textContent = result.kind === "limit" ? "Stopped by a limit" : `Failed (${result.kind} error)`;
    errorLine = result.line ?? null;
  }
  output.innerHTML = html;
  render();
}

function formatMs(ms) {
  return ms < 1 ? "under 1 ms" : ms < 1000 ? `${ms.toFixed(1)} ms` : `${(ms / 1000).toFixed(2)} s`;
}

output.addEventListener("click", (event) => {
  const target = event.target.closest(".jump");
  if (!target) return;
  goTo(Number(target.dataset.line), Number(target.dataset.col));
});

function goTo(line, col) {
  const lines = code.value.split("\n");
  let offset = 0;
  for (let i = 0; i < line - 1 && i < lines.length; i++) offset += lines[i].length + 1;
  offset += Math.max(0, (col || 1) - 1);
  code.focus();
  code.setSelectionRange(offset, offset);
  code.scrollTop = Math.max(0, (line - 4) * lineHeight());
  updateCursor();
  syncScroll();
}

runButton.addEventListener("click", run);
bytecodeButton.addEventListener("click", () => {
  const on = bytecodeButton.getAttribute("aria-pressed") !== "true";
  bytecodeButton.setAttribute("aria-pressed", String(on));
  run();
});

// ----- examples and sharing ---------------------------------------------------

const placeholder = document.createElement("option");
placeholder.value = "";
placeholder.textContent = "Load an example…";
examples.append(placeholder);
for (const [i, example] of EXAMPLES.entries()) {
  const option = document.createElement("option");
  option.value = String(i);
  option.textContent = example.title;
  examples.append(option);
}
examples.addEventListener("change", () => {
  const example = examples.value === "" ? null : EXAMPLES[Number(examples.value)];
  examples.value = "";
  if (example) {
    setCode(example.code);
    run();
  }
});

function encode(text) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function decode(data) {
  const binary = atob(data.replace(/-/g, "+").replace(/_/g, "/"));
  return new TextDecoder().decode(Uint8Array.from(binary, (c) => c.charCodeAt(0)));
}

function showToast(message) {
  toast.textContent = message;
  toast.hidden = false;
  clearTimeout(showToast.timer);
  showToast.timer = setTimeout(() => (toast.hidden = true), 2200);
}

$("share").addEventListener("click", async () => {
  const url = `${location.origin}${location.pathname}#code=${encode(code.value)}`;
  history.replaceState(null, "", url);
  try {
    await navigator.clipboard.writeText(url);
    showToast("Link copied");
  } catch {
    showToast("Link is in the address bar");
  }
});

function initialCode() {
  const match = /^#code=(.+)$/.exec(location.hash);
  if (match) {
    try {
      return decode(match[1]);
    } catch {
      showToast("That link's code couldn't be read");
    }
  }
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return saved;
  } catch {
    // No storage: start from the first example.
  }
  return EXAMPLES[0].code;
}

runButton.disabled = true;
setCode(initialCode());
startWorker();
window.addEventListener("resize", render);
document.fonts?.ready.then(render);
