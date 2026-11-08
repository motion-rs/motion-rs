const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");

const roots = [
  "README.md",
  "gallery/README.md",
  "gallery/assets/README.md",
  "tests/fixtures/fonts/README.md",
  "docs",
  ".github",
];

// Design records and dated reviews are not compiled: links are checked, fence tags are not.
const looseFences = (relative) =>
  relative === "docs/roadmap.md" || relative.startsWith("docs/reviews/");

function markdownFiles(entry) {
  const file = path.join(root, entry);
  if (!fs.existsSync(file)) return [];
  if (fs.statSync(file).isFile()) return entry.endsWith(".md") ? [entry] : [];
  return fs
    .readdirSync(file, { withFileTypes: true })
    .flatMap((child) =>
      child.name === "node_modules"
        ? []
        : markdownFiles(path.posix.join(entry, child.name)),
    );
}

// GitHub heading anchors: lowercase, drop punctuation, spaces to hyphens, -N for repeats.
function anchors(source) {
  const seen = new Map();
  const result = new Set();
  let fenced = false;
  for (const line of source.split(/\r?\n/)) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const heading = !fenced && /^#{1,6}\s+(.*?)\s*#*\s*$/.exec(line);
    if (!heading) continue;
    const base = heading[1]
      .replace(/`/g, "")
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      .toLowerCase()
      .replace(/[^\p{L}\p{N}_\- ]/gu, "")
      .replace(/ /g, "-");
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    result.add(count ? `${base}-${count}` : base);
  }
  return result;
}

function analyze(relative, source, exists, headings) {
  const issues = [];
  let fenced = false;
  const prose = [];
  source.split(/\r?\n/).forEach((line, index) => {
    const fence = /^\s*(```|~~~)(\S*)/.exec(line);
    if (fence) {
      if (!fenced && !fence[2] && !looseFences(relative))
        issues.push(`${relative}:${index + 1}: Untagged code fence`);
      fenced = !fenced;
      return;
    }
    if (!fenced) prose.push([index + 1, line.replace(/`[^`]*`/g, "")]);
  });
  for (const [line, text] of prose) {
    for (const match of text.matchAll(/\]\(([^)\s]+)\)/g)) {
      const target = match[1];
      if (/^(?:[a-z][a-z0-9+.-]*:|#?$)/i.test(target) && !target.startsWith("#"))
        continue;
      const [file, anchor] = target.split("#");
      const resolved = file
        ? path.posix.normalize(path.posix.join(path.posix.dirname(relative), file))
        : relative;
      if (!exists(resolved)) {
        issues.push(`${relative}:${line}: Broken link ${target}`);
        continue;
      }
      if (anchor && resolved.endsWith(".md")) {
        const available = headings(resolved);
        if (!available.has(anchor))
          issues.push(`${relative}:${line}: Missing anchor ${target}`);
      }
    }
  }
  return issues;
}

const files = new Map([
  ["a/x.md", "# Title\n\n## Two words\n\n## Two words\n"],
  ["a/y.md", "[ok](x.md#two-words) [dup](x.md#two-words-1)\n"],
]);
const fakeExists = (file) => files.has(file);
const fakeHeadings = (file) => anchors(files.get(file));
assert.deepEqual(analyze("a/y.md", files.get("a/y.md"), fakeExists, fakeHeadings), []);
assert.equal(
  analyze("a/y.md", "[x](x.md#nope) [y](missing.md)\n", fakeExists, fakeHeadings).length,
  2,
);
assert.equal(
  analyze("a/y.md", "```\ntext\n```\n```sh\nls\n```\n", fakeExists, fakeHeadings).length,
  1,
);
assert.deepEqual(
  analyze("a/y.md", "`[x](gone.md)` and [web](https://example.com)\n", fakeExists, fakeHeadings),
  [],
);

if (process.argv.includes("--self-test")) {
  console.log("Documentation checks self-test passed.");
  process.exit(0);
}

const exists = (file) => fs.existsSync(path.join(root, file));
const cache = new Map();
const headings = (file) => {
  if (!cache.has(file))
    cache.set(file, anchors(fs.readFileSync(path.join(root, file), "utf8")));
  return cache.get(file);
};
const documents = [...new Set(roots.flatMap(markdownFiles))].sort();
const failures = documents.flatMap((relative) =>
  analyze(
    relative,
    fs.readFileSync(path.join(root, relative), "utf8"),
    exists,
    headings,
  ),
);
assert.equal(failures.length, 0, failures.join("\n"));
console.log(`Documentation checks passed (${documents.length} files).`);
