const fs = require("node:fs");
const cp = require("node:child_process");
const assert = require("node:assert/strict");
const snapshotFile = "target/tmp/rustdoc-format-before.json";
const git = (...args) => cp.execFileSync("git", args, { encoding: "utf8" });
const blockPattern = () =>
  /(^([ \t]*\/\/[/!]) ?```([^\n]*)\n)([\s\S]*?)(^\2 ?```[ \t]*(?=\n|$))/gm;
const decode = (body, prefix) =>
  body
    .slice(0, -1)
    .split("\n")
    .map((line) => {
      assert.ok(
        line.startsWith(prefix),
        "Non-documentation line inside fenced example",
      );
      return line.slice(prefix.length).replace(/^ /, "");
    });

function tokens(source) {
  const result = [];
  while (source.length) {
    const whitespace = /^\s+/.exec(source);
    if (whitespace) {
      source = source.slice(whitespace[0].length);
      continue;
    }
    let token;
    const raw = /^(?:b|c)?r(#+)?"/.exec(source);
    if (raw) {
      const end = source.indexOf('"' + (raw[1] || ""), raw[0].length);
      assert.ok(end >= 0, "Unterminated raw string");
      token = source.slice(0, end + 1 + (raw[1] || "").length);
    } else if (source.startsWith("/*")) {
      let depth = 1;
      let end = 2;
      while (depth && end < source.length) {
        if (source.startsWith("/*", end)) {
          depth++;
          end += 2;
        } else if (source.startsWith("*/", end)) {
          depth--;
          end += 2;
        } else end++;
      }
      assert.equal(depth, 0, "Unterminated block comment");
      token = source.slice(0, end);
    } else {
      token =
        /^(?:\/\/[^\n]*|(?:b|c)?"(?:\\[\s\S]|[^"\\])*"|b?'(?:\\(?:u\{[\da-fA-F_]+\}|x[\da-fA-F]{2}|.)|[^'\\\n])'|[A-Za-z_][A-Za-z_0-9]*|[0-9][A-Za-z_0-9]*|[^\s])/u.exec(
          source,
        )?.[0];
    }
    assert.ok(token, "Unsupported Rust token");
    result.push(token.trimEnd());
    source = source.slice(token.length);
  }
  const filtered = result.filter(
    (token, index) =>
      token !== "," || ![")", "]", "}"].includes(result[index + 1]),
  );
  const closureBraces = new Set();
  filtered.forEach((token, index) => {
    if (token !== "{" || filtered[index - 1] !== "|") return;
    let depth = 1;
    let end = index + 1;
    let statements = false;
    for (; end < filtered.length && depth; end++) {
      if (filtered[end] === "{") depth++;
      else if (filtered[end] === "}") depth--;
      else if (depth === 1 && filtered[end] === ";") statements = true;
    }
    if (depth === 0 && !statements) {
      closureBraces.add(index);
      closureBraces.add(end - 1);
    }
  });
  return filtered.filter((_, index) => !closureBraces.has(index));
}

function format(body, prefix) {
  const original = decode(body, prefix);
  const hidden = [];
  const source = original
    .map((line) => {
      if (!/^#(?: |$)/.test(line)) return line;
      const index = hidden.push(line) - 1;
      return (
        "// RUSTDOC_START_" +
        index +
        "\n" +
        line.replace(/^# ?/, "") +
        "\n// RUSTDOC_END_" +
        index
      );
    })
    .join("\n");
  const input =
    "fn rustdoc_example() {\n" +
    source
      .split("\n")
      .map((line) => "    " + line)
      .join("\n") +
    "\n}\n";
  const output = cp.execFileSync(
    "rustfmt",
    [
      "--emit",
      "stdout",
      "--edition",
      "2024",
      "--config",
      "skip_children=true,reorder_imports=false,style_edition=2024",
    ],
    { input, encoding: "utf8" },
  );
  const lines = output.trimEnd().split("\n");
  assert.equal(lines.shift(), "fn rustdoc_example() {");
  assert.equal(lines.pop(), "}");
  let hiddenMain = false;
  let activeHidden = null;
  const seen = [];
  const formatted = [];
  lines.forEach((line) => {
    line = line.replace(/^    /, "");
    const start = /^\s*\/\/ RUSTDOC_START_(\d+)$/.exec(line);
    if (start) {
      assert.equal(activeHidden, null);
      activeHidden = Number(start[1]);
      return;
    }
    const end = /^\s*\/\/ RUSTDOC_END_(\d+)$/.exec(line);
    if (end) {
      const index = Number(end[1]);
      assert.equal(activeHidden, index);
      activeHidden = null;
      const originalLine = hidden[index];
      seen.push(index);
      if (/^# fn main\(/.test(originalLine)) hiddenMain = true;
      if (originalLine === "# }") hiddenMain = false;
      formatted.push(originalLine);
      return;
    }
    if (activeHidden === null)
      formatted.push(hiddenMain ? line.replace(/^    /, "") : line);
  });
  assert.equal(activeHidden, null);
  assert.deepEqual(
    seen,
    hidden.map((_, index) => index),
    "Hidden lines changed or moved",
  );
  assert.ok(
    JSON.stringify(tokens(formatted.join("\n"))) ===
    JSON.stringify(tokens(original.join("\n"))),
    "Rust tokens changed beyond trailing commas and expression-closure braces",
  );
  return (
    formatted.map((line) => prefix + (line ? " " + line : "")).join("\n") + "\n"
  );
}

function transform(text, action) {
  return text.replace(
    blockPattern(),
    (match, opening, prefix, flags, body, closing) => {
      assert.ok(
        ["", "no_run", "compile_fail"].includes(flags),
        "Unsupported fence: " + flags,
      );
      return opening + action(body, prefix) + closing;
    },
  );
}

if (process.argv.includes("--verify")) {
  const snapshot = JSON.parse(fs.readFileSync(snapshotFile, "utf8"));
  let blocks = 0;
  let changed = 0;
  for (const [file, before] of Object.entries(snapshot.files)) {
    const after = fs.readFileSync(file, "utf8");
    const oldBodies = [];
    const mask = (text, bodies) =>
      transform(text, (body, prefix) => {
        bodies.push({ body, prefix });
        return "<RUSTDOC_CODE>\n";
      });
    const newBodies = [];
    assert.equal(
      mask(after, newBodies),
      mask(before, oldBodies),
      file + ": prose or implementation changed",
    );
    assert.equal(newBodies.length, oldBodies.length);
    newBodies.forEach(({ body, prefix }, index) => {
      const original = decode(oldBodies[index].body, prefix);
      const current = decode(body, prefix);
      assert.deepEqual(
        current.filter((line) => /^#(?: |$)/.test(line)),
        original.filter((line) => /^#(?: |$)/.test(line)),
        file + ": hidden lines changed",
      );
      assert.deepEqual(
        tokens(current.join("\n")),
        tokens(original.join("\n")),
        file + ": token mismatch",
      );
      assert.equal(
        format(body, prefix),
        body,
        file + ": formatting is not idempotent",
      );
      blocks++;
    });
    if (after !== before) changed++;
  }
  assert.equal(
    git("ls-files", "--stage"),
    snapshot.index,
    "Index changed during formatting",
  );
  console.log(
    JSON.stringify(
      {
        checkedFiles: Object.keys(snapshot.files).length,
        checkedBlocks: blocks,
        changedFilesAfterProbe: changed,
        proseAndImplementation: "unchanged",
        hiddenLines: "unchanged",
        tokens:
          "preserved except trailing commas and expression-closure braces",
        rustfmt: "idempotent",
        index: "unchanged",
      },
      null,
      2,
    ),
  );
} else {
  const files = git("ls-files", "-z", "--", "crates", "gallery", "examples")
    .split("\0")
    .filter((file) => file.endsWith(".rs") && fs.existsSync(file));
  const snapshot = { index: git("ls-files", "--stage"), files: {} };
  const edits = [];
  let blocks = 0;
  for (const file of files) {
    const before = fs.readFileSync(file, "utf8");
    if (!blockPattern().test(before)) continue;
    snapshot.files[file] = before;
    const after = transform(before, (body, prefix) => {
      blocks++;
      return format(body, prefix);
    });
    if (after !== before) edits.push({ file, before, after });
  }
  console.log(
    JSON.stringify(
      {
        checkedFiles: Object.keys(snapshot.files).length,
        blocks,
        changedFiles: edits.map((edit) => edit.file),
      },
      null,
      2,
    ),
  );
  if (process.argv.includes("--write")) {
    assert.ok(!fs.existsSync(snapshotFile), "Snapshot already exists");
    fs.writeFileSync(snapshotFile, JSON.stringify(snapshot));
    for (const { file, before, after } of edits) {
      assert.equal(
        fs.readFileSync(file, "utf8"),
        before,
        file + ": concurrent modification",
      );
      fs.writeFileSync(file, after);
    }
  }
}
