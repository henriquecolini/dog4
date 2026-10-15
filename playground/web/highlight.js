// Syntax highlighting for dog4 source, as HTML spans with `t-*` classes.
//
// This is a highlighter, not a parser: it follows dog4's lexical rules closely
// enough for display (commands at statement starts, `$` paths and
// interpolation, map keys, parameter types, bare literals) and never fails.

const KEYWORDS = new Set([
  "fn", "if", "else", "while", "for", "in", "split", "try", "catch", "throw",
  "return", "let", "use", "as", "mod", "pub", "break", "continue", "clear",
  "true", "false", "null",
]);
const TYPES = new Set(["num", "str", "arr", "map", "fn", "bool", "any", "null"]);

const escapeHtml = (s) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);
const span = (cls, text) => `<span class="t-${cls}">${escapeHtml(text)}</span>`;
const isIdentStart = (c) => /[\p{L}_]/u.test(c);
const isIdentChar = (c) => /[\p{L}_0-9]/u.test(c);
const isNumber = (word) => /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(word);

/** End index of a variable path (`name.field[index]`) starting at `i`. */
function pathEnd(src, i) {
  while (i < src.length && isIdentChar(src[i])) i++;
  for (;;) {
    if (src[i] === "." && isIdentChar(src[i + 1] || "")) {
      i++;
      while (i < src.length && isIdentChar(src[i])) i++;
    } else if (src[i] === "[") {
      let depth = 0;
      for (; i < src.length; i++) {
        if (src[i] === "[") depth++;
        if (src[i] === "]" && --depth === 0) {
          i++;
          break;
        }
      }
    } else if (src[i] === "(") {
      // `$f(args)`
      i = matching(src, i, "(", ")") + 1;
    } else {
      return i;
    }
  }
}

function matching(src, open, a, b) {
  let depth = 0;
  for (let j = open; j < src.length; j++) {
    if (src[j] === a) depth++;
    if (src[j] === b && --depth === 0) return j;
  }
  return src.length - 1;
}

/** Highlights a variable path, recursing into index and call arguments. */
function highlightPath(text) {
  let out = "";
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === "[" || c === "(") {
      const close = matching(text, i, c, c === "[" ? "]" : ")");
      out += span("punct", c) + highlight(text.slice(i + 1, close)) + span("punct", text[close] || "");
      i = close + 1;
    } else {
      let j = i;
      while (j < text.length && text[j] !== "[" && text[j] !== "(") j++;
      out += span("var", text.slice(i, j));
      i = j;
    }
  }
  return out;
}

/** A `$` form at `src[i]`, as `[html, end]`, or null when `$` is plain text. */
function dollar(src, i, inString) {
  const next = src[i + 1] || "";
  if (next === "%" && isIdentStart(src[i + 2] || "")) {
    const end = pathEnd(src, i + 2);
    return [span("var", "$%") + highlightPath(src.slice(i + 2, end)), end];
  }
  if (isIdentStart(next)) {
    let end = pathEnd(src, i + 1);
    if (inString) {
      // No calls inside strings: `"$f(1)"` is `$f` then text.
      let j = i + 1;
      while (j < src.length && isIdentChar(src[j])) j++;
      for (;;) {
        if (src[j] === "." && isIdentChar(src[j + 1] || "")) {
          j++;
          while (j < src.length && isIdentChar(src[j])) j++;
        } else if (src[j] === "[") {
          j = matching(src, j, "[", "]") + 1;
        } else break;
      }
      end = j;
    }
    return [span("var", "$") + highlightPath(src.slice(i + 1, end)), end];
  }
  if (next === "{") {
    const close = src.indexOf("}", i);
    const end = close < 0 ? src.length : close + 1;
    return [span("var", src.slice(i, end)), end];
  }
  if (next === "(") {
    const close = matching(src, i + 1, "(", ")");
    return [span("var", "$(") + highlight(src.slice(i + 2, close)) + span("var", ")"), close + 1];
  }
  return null;
}

/** Returns `src` as highlighted HTML. */
export function highlight(src) {
  let out = "";
  let i = 0;
  let atStatement = true;
  let afterFn = false;
  let paramDepth = 0;
  let expectType = false;
  const brackets = [];
  while (i < src.length) {
    const c = src[i];
    const startsToken = i === 0 || /[\s;{}()[\],]/.test(src[i - 1]);
    if (c === "/" && src[i + 1] === "/" && startsToken) {
      let j = src.indexOf("\n", i);
      if (j < 0) j = src.length;
      out += span("com", src.slice(i, j));
      i = j;
      continue;
    }
    if (c === "/" && src[i + 1] === "*" && startsToken) {
      let depth = 0;
      let j = i;
      for (; j < src.length; j++) {
        if (src.startsWith("/*", j)) { depth++; j++; }
        else if (src.startsWith("*/", j)) { depth--; j++; if (depth === 0) { j++; break; } }
      }
      out += span("com", src.slice(i, j));
      i = j;
      continue;
    }
    if (/\s/.test(c)) {
      out += c;
      i++;
      continue;
    }
    if (c === "\\") {
      out += span("esc", src.slice(i, i + 2));
      i += 2;
      atStatement = false;
      continue;
    }
    if (c === '"') {
      let j = i + 1;
      let piece = '"';
      out += '<span class="t-str">';
      while (j < src.length && src[j] !== '"') {
        if (src[j] === "\\") {
          out += escapeHtml(piece) + span("esc", src.slice(j, j + 2));
          piece = "";
          j += 2;
          continue;
        }
        if (src[j] === "$") {
          const found = dollar(src, j, true);
          if (found) {
            out += escapeHtml(piece) + found[0];
            piece = "";
            j = found[1];
            continue;
          }
        }
        piece += src[j++];
      }
      out += escapeHtml(piece + (j < src.length ? '"' : "")) + "</span>";
      i = j + 1;
      atStatement = false;
      continue;
    }
    if (c === "'" && startsToken) {
      const j = src.indexOf("'", i + 1);
      const end = j < 0 ? src.length : j + 1;
      out += span("str", src.slice(i, end));
      i = end;
      atStatement = false;
      continue;
    }
    if (c === "$") {
      const found = dollar(src, i, false);
      if (found) {
        out += found[0];
        i = found[1];
        atStatement = false;
        continue;
      }
    }
    if (c === "&" && paramDepth > 0) {
      out += span("type", "&");
      expectType = true;
      i++;
      continue;
    }
    if (c === "&" && isIdentStart(src[i + 1] || "")) {
      const end = pathEnd(src, i + 1);
      out += span("var", "&") + highlightPath(src.slice(i + 1, end));
      i = end;
      atStatement = false;
      continue;
    }
    if (startsToken && /[-\d]/.test(c)) {
      const word = /^[^\s;{}()[\],=]+/.exec(src.slice(i))[0];
      if (isNumber(word)) {
        out += span("num", word);
        i += word.length;
        atStatement = false;
        continue;
      }
    }
    if (isIdentStart(c)) {
      let j = i;
      while (j < src.length && isIdentChar(src[j])) j++;
      const word = src.slice(i, j);
      const rest = src.slice(j);
      const wholeWord = !/^[^\s;{}()[\],=]/.test(rest) || /^[.(]/.test(rest);
      let cls = null;
      if (expectType && TYPES.has(word)) {
        cls = "type";
      } else if (afterFn) {
        cls = "fname";
        afterFn = false;
      } else if (KEYWORDS.has(word) && (wholeWord || /^[({[$"']/.test(rest))) {
        cls = "kw";
        if (word === "fn") afterFn = /^\s+[\p{L}_]/u.test(rest);
      } else if (brackets[brackets.length - 1] === "[" && /^\s*=(?!=)/.test(rest)) {
        cls = "key";
      } else if (atStatement && !/^(\.[\p{L}_0-9]+|\[[^\]]*\])*\s*[-+*/]?=(?!=)/u.test(rest)) {
        cls = "cmd";
      } else if (atStatement) {
        cls = "assign";
      }
      expectType = false;
      out += cls ? span(cls, word) : escapeHtml(word);
      i = j;
      atStatement = false;
      continue;
    }
    const op = /^(==|!=|<=|>=|&&|\|\||\?\?|\.\.|\+=|-=|\*=|\/=|=)/.exec(src.slice(i));
    if (op && (op[0] !== "=" || /\s/.test(src[i - 1] || " ") || brackets.length)) {
      out += span("op", op[0]);
      i += op[0].length;
      atStatement = false;
      continue;
    }
    if (/[+\-*/%<>!|?]/.test(c) && /[\s(]/.test(src[i - 1] || " ") && /[\s($\d]/.test(src[i + 1] || " ")) {
      out += span("op", c);
      i++;
      continue;
    }
    if ("{}[]();,:".includes(c)) {
      if (c === "(" && /fn(\s+[\p{L}_][\p{L}_0-9]*)?\s*$/u.test(src.slice(0, i))) paramDepth = 1;
      else if (c === "(" && paramDepth > 0) paramDepth++;
      if (c === ")" && paramDepth > 0) paramDepth--;
      if (c === "(" || c === "[" || c === "{") brackets.push(c);
      if (c === ")" || c === "]" || c === "}") brackets.pop();
      expectType = c === ":" && paramDepth > 0;
      out += span("punct", c);
      atStatement = c === ";" || c === "{" || (c === "}" && /^[ \t]*(\n|$)/.test(src.slice(i + 1)));
      i++;
      continue;
    }
    out += escapeHtml(c);
    atStatement = false;
    i++;
  }
  return out;
}
