const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

function tokenize(source) {
  const tokens = [];
  const stack = [];
  const pairs = new Map();
  let offset = 0;
  let line = 0;
  while (offset < source.length) {
    if (/\s/u.test(source[offset])) {
      if (source[offset] === "\n") line += 1;
      offset += 1;
      continue;
    }
    const start = offset;
    const startLine = line;
    let kind = "punctuation";
    if (source.startsWith("//", offset)) {
      kind = "comment";
      const end = source.indexOf("\n", offset);
      offset = end < 0 ? source.length : end;
    } else if (source.startsWith("/*", offset)) {
      kind = "comment";
      let depth = 1;
      offset += 2;
      while (offset < source.length && depth > 0) {
        if (source.startsWith("/*", offset)) {
          depth += 1;
          offset += 2;
        } else if (source.startsWith("*/", offset)) {
          depth -= 1;
          offset += 2;
        } else offset += 1;
      }
      assert.equal(depth, 0, "Unterminated block comment");
    } else {
      const remaining = source.slice(offset);
      const raw = /^(?:br|cr|r)(#*)"/u.exec(remaining);
      const character =
        /^'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^])|[^'\\\r\n])'/u.exec(
          remaining,
        );
      const word = /^[\p{ID_Start}_][\p{ID_Continue}]*/u.exec(remaining);
      if (raw) {
        kind = "literal";
        const closing = '"' + raw[1];
        const end = source.indexOf(closing, offset + raw[0].length);
        assert.ok(end >= 0, "Unterminated raw string");
        offset = end + closing.length;
      } else if (source[offset] === '"') {
        kind = "literal";
        offset += 1;
        while (offset < source.length) {
          if (source[offset] === "\\") offset += 2;
          else if (source[offset++] === '"') break;
        }
      } else if (character) {
        kind = "literal";
        offset += character[0].length;
      } else if (word) {
        kind = "word";
        offset += word[0].length;
      } else {
        offset += 1;
      }
    }
    const text = source.slice(start, offset);
    const token = {
      kind,
      text,
      start,
      end: offset,
      line: startLine,
      endLine: startLine + (text.match(/\n/gu) || []).length,
      parent: stack.at(-1)?.index ?? -1,
      protected: stack.at(-1)?.protected ?? false,
    };
    line = token.endLine;
    const index = tokens.length;
    if (kind === "punctuation" && "([{".includes(text)) {
      const previous = tokens.at(-1);
      stack.push({
        index,
        text,
        protected:
          token.protected ||
          text === "[" ||
          previous?.text === "!" ||
          tokens.at(-2)?.text === "!",
      });
    } else if (kind === "punctuation" && ")]}".includes(text)) {
      const open = stack.pop();
      assert.ok(
        open && "([{".indexOf(open.text) === ")]}".indexOf(text),
        "Unbalanced delimiters",
      );
      pairs.set(index, open.index);
      pairs.set(open.index, index);
    }
    tokens.push(token);
  }
  assert.equal(stack.length, 0, "Unclosed delimiters");
  return { tokens, pairs };
}

function separateItems(source) {
  const { tokens, pairs } = tokenize(source);
  const lines = source.split("\n");
  const lineStarts = [0];
  for (let index = 0; index < source.length; index += 1) {
    if (source[index] === "\n") lineStarts.push(index + 1);
  }
  const ownLine = (token) =>
    source.slice(lineStarts[token.line], token.start).trim() === "";
  const insertions = new Set();
  const useEnds = new Set();
  // Consecutive `mod name;` declarations stay contiguous so rustfmt sorts them.
  const modEnds = new Set();
  const removals = new Set();
  const itemKinds = new Set([
    "fn",
    "struct",
    "enum",
    "trait",
    "impl",
    "mod",
    "type",
    "const",
    "static",
    "union",
    "extern",
    "macro",
    "macro_rules",
    "use",
  ]);
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token.kind !== "word" || token.protected || !ownLine(token)) continue;
    if (token.parent >= 0 && tokens[token.parent].text !== "{") continue;
    let cursor = index;
    if (tokens[cursor]?.text === "pub") {
      cursor += 1;
      if (tokens[cursor]?.text === "(") cursor = pairs.get(cursor) + 1;
    }
    while (
      ["unsafe", "async", "default", "auto", "safe"].includes(
        tokens[cursor]?.text,
      )
    )
      cursor += 1;
    if (
      tokens[cursor]?.text === "const" &&
      ["fn", "unsafe", "async"].includes(tokens[cursor + 1]?.text)
    ) {
      cursor += 1;
      while (["unsafe", "async"].includes(tokens[cursor]?.text)) cursor += 1;
    }
    const kind = tokens[cursor]?.text;
    if (!itemKinds.has(kind)) continue;
    if (kind === "const" && tokens[cursor + 1]?.text === "{") continue;
    if (kind === "use") {
      for (let end = cursor + 1; end < tokens.length; end += 1) {
        if (tokens[end].text === ";" && tokens[end].parent === token.parent) {
          useEnds.add(end);
          break;
        }
      }
    }
    const declaration =
      kind === "mod" &&
      tokens[cursor + 1]?.kind === "word" &&
      tokens[cursor + 2]?.text === ";";
    if (declaration) modEnds.add(cursor + 2);
    let prefix = index;
    while (prefix > 0) {
      const previous = tokens[prefix - 1];
      if (previous.kind === "comment" && ownLine(previous)) {
        prefix -= 1;
        continue;
      }
      if (previous.text === "]") {
        const open = pairs.get(prefix - 1);
        if (
          open > 0 &&
          tokens[open - 1].text === "#" &&
          ownLine(tokens[open - 1])
        ) {
          prefix = open - 1;
          continue;
        }
      }
      break;
    }
    let previousIndex = prefix - 1;
    while (previousIndex >= 0 && tokens[previousIndex].kind === "comment")
      previousIndex -= 1;
    const previous = tokens[previousIndex];
    if (!previous || ![";", "}"].includes(previous.text)) continue;
    if (kind === "use" && useEnds.has(previousIndex)) continue;
    const startLine = tokens[prefix].line;
    if (declaration && modEnds.has(previousIndex)) {
      for (let line = previous.endLine + 1; line < startLine; line += 1)
        if (lines[line].trim() === "") removals.add(line);
      continue;
    }
    if (
      startLine > previous.endLine &&
      startLine > 0 &&
      lines[startLine - 1].trim() !== ""
    ) {
      insertions.add(startLine);
    }
  }
  const formatted = lines
    .flatMap((value, index) =>
      removals.has(index)
        ? []
        : [(insertions.has(index) ? "\n" : "") + value],
    )
    .join("\n");
  assert.deepEqual(
    tokenize(formatted).tokens.map((token) => [token.kind, token.text]),
    tokens.map((token) => [token.kind, token.text]),
    "Declaration spacing changed Rust tokens",
  );
  return { formatted, count: insertions.size + removals.size };
}

function selfTest() {
  const cases = [
    [
      "use a::A;\nuse b::B;\nstruct Foo;\nimpl Foo {\n    fn first() {}\n    /// Second.\n    #[cfg(test)]\n    fn second() {}\n}\n",
      "use a::A;\nuse b::B;\n\nstruct Foo;\n\nimpl Foo {\n    fn first() {}\n\n    /// Second.\n    #[cfg(test)]\n    fn second() {}\n}\n",
    ],
    [
      'fn first() {}\n#[cfg(any(\n    feature = "gpu",\n    test,\n))]\nfn second() {}\n',
      'fn first() {}\n\n#[cfg(any(\n    feature = "gpu",\n    test,\n))]\nfn second() {}\n',
    ],
    [
      'const SOURCE: &str = r###"\n}\nfn fake() {}\n"###;\nfn real() {}\n',
      'const SOURCE: &str = r###"\n}\nfn fake() {}\n"###;\n\nfn real() {}\n',
    ],
    [
      "fn first() {}\n/* nested /* fn fake() {} */ comment */\nfn second() {}\n",
      "fn first() {}\n\n/* nested /* fn fake() {} */ comment */\nfn second() {}\n",
    ],
    [
      "macro_rules! example { () => {\nfn first() {}\nfn second() {}\n}; }\nfn actual() {}\n",
      "macro_rules! example { () => {\nfn first() {}\nfn second() {}\n}; }\n\nfn actual() {}\n",
    ],
    [
      "fn lifetime<'source>(value: &'source str) {\n    let brace = '}';\n    let other = '{';\n}\nfn after() {}\n",
      "fn lifetime<'source>(value: &'source str) {\n    let brace = '}';\n    let other = '{';\n}\n\nfn after() {}\n",
    ],
    [
      "trait Example {\n    type Value;\n    const COUNT: usize;\n    fn first();\n    fn second();\n}\n",
      "trait Example {\n    type Value;\n\n    const COUNT: usize;\n\n    fn first();\n\n    fn second();\n}\n",
    ],
    [
      "mod b;\n\n#[cfg(test)]\nmod a;\n\npub mod c;\npub use c::C;\nmod inline {}\n",
      "mod b;\n#[cfg(test)]\nmod a;\npub mod c;\n\npub use c::C;\n\nmod inline {}\n",
    ],
  ];
  for (const [source, expected] of cases) {
    const result = separateItems(source).formatted;
    assert.equal(result, expected);
    assert.equal(separateItems(result).count, 0);
  }
  console.log(`Declaration-spacing self-checks passed: ${cases.length}.`);
}

function rustFiles(root) {
  const files = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name);
      const relative = path.relative(root, filename).split(path.sep).join("/");
      if (entry.isDirectory()) {
        if (
          ["target", ".git", "node_modules", ".motion"].includes(entry.name) ||
          relative === "gallery/reference"
        )
          continue;
        walk(filename);
      } else if (entry.isFile() && entry.name.endsWith(".rs"))
        files.push(filename);
    }
  }
  walk(root);
  return files.sort();
}

function rustfmt(files, root, check = false) {
  const args = [
    "--config-path",
    path.join(root, "rustfmt.toml"),
    "--config",
    "skip_children=true",
  ];
  if (check) args.push("--check");
  for (let offset = 0; offset < files.length; offset += 30) {
    const result = spawnSync(
      "rustfmt",
      [...args, ...files.slice(offset, offset + 30)],
      { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
    );
    if (result.status !== 0 || result.stderr) {
      if (result.stdout) console.log(result.stdout);
      if (result.stderr) console.error(result.stderr);
      assert.equal(result.status, 0, "rustfmt failed");
    }
  }
}

function main() {
  selfTest();
  if (process.argv.includes("--self-test")) return;
  const root = process.cwd();
  assert.equal(path.basename(root), "motion-rs");
  const files = rustFiles(root);
  const check = process.argv.includes("--check");
  if (check) {
    rustfmt(files, root, true);
    let missing = 0;
    for (const filename of files) {
      const result = separateItems(fs.readFileSync(filename, "utf8"));
      if (result.count) {
        console.log(path.relative(root, filename) + ": " + result.count);
        missing += result.count;
      }
    }
    assert.equal(missing, 0, "Missing declaration spacing");
    console.log(
      `Formatting and declaration spacing passed: ${files.length} Rust files.`,
    );
    return;
  }
  const originals = new Map(
    files.map((filename) => [filename, fs.readFileSync(filename, "utf8")]),
  );
  const archive = path.join(root, "gallery/reference");
  const archiveBefore = fs.existsSync(archive)
    ? require("node:crypto").createHash("sha256")
    : null;
  const archiveFiles = [];
  function hashArchive(directory, hash) {
    for (const entry of fs
      .readdirSync(directory, { withFileTypes: true })
      .sort((left, right) => left.name.localeCompare(right.name))) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) hashArchive(filename, hash);
      else if (entry.isFile()) {
        hash.update(path.relative(archive, filename));
        hash.update(fs.readFileSync(filename));
        archiveFiles.push(filename);
      }
    }
  }
  if (archiveBefore) hashArchive(archive, archiveBefore);
  const archiveDigest = archiveBefore?.digest("hex");
  rustfmt(files, root);
  let inserted = 0;
  for (const filename of files) {
    const source = fs.readFileSync(filename, "utf8");
    const result = separateItems(source);
    if (result.count) fs.writeFileSync(filename, result.formatted);
    inserted += result.count;
  }
  rustfmt(files, root);
  rustfmt(files, root, true);
  for (const filename of files) {
    assert.equal(
      separateItems(fs.readFileSync(filename, "utf8")).count,
      0,
      path.relative(root, filename),
    );
  }
  if (archiveBefore) {
    const archiveAfter = require("node:crypto").createHash("sha256");
    hashArchive(archive, archiveAfter);
    assert.equal(
      archiveAfter.digest("hex"),
      archiveDigest,
      "Historical archive changed",
    );
  }
  const changed = files.filter(
    (filename) => fs.readFileSync(filename, "utf8") !== originals.get(filename),
  );
  console.log(
    `Formatted ${files.length} Rust files; changed ${changed.length}; inserted ${inserted} declaration separators.`,
  );
  console.log(
    `Historical archive preserved: ${archiveFiles.length / 2} files; SHA256 ${archiveDigest}.`,
  );
  console.log(
    "rustfmt --check, token preservation for blank-line edits, and spacing idempotence passed.",
  );
}

if (require.main === module) main();

module.exports = { tokenize };
