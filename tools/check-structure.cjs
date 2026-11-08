const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { tokenize } = require("./rust-format.cjs");

const root = path.resolve(__dirname, "..");

const maxImplementationLines = 800;

function isTestSource(relative) {
  return (
    /(?:^|\/)tests\.rs$/.test(relative) ||
    /_tests\.rs$/.test(relative) ||
    /\/tests\//.test(relative)
  );
}

function organizationIssue(relative, source) {
  if (isTestSource(relative)) return null;
  const lines = source.trimEnd().split(/\r?\n/).length;
  if (lines > maxImplementationLines)
    return `Implementation has ${lines} lines (limit ${maxImplementationLines}); split by responsibility`;
  return null;
}

assert(
  organizationIssue(
    "crates/motion-render/src/gpu/execute.rs",
    "fn example() {}\n".repeat(801),
  ),
);
assert.equal(
  organizationIssue(
    "crates/motion-render/src/gpu/execute.rs",
    "fn example() {}\n".repeat(800),
  ),
  null,
);
assert.equal(
  organizationIssue(
    "crates/motion-render/src/gpu/execute/tests.rs",
    "fn example() {}\n".repeat(801),
  ),
  null,
);

function documentationIssue(source) {
  const first = source.split(/\r?\n/, 1)[0];
  if (/^\/\/! [a-z]/.test(first))
    return "Module summary must start with an uppercase letter";
  if (/^\/\/! .*use\s+(?:crate|std|super)::/.test(first))
    return "Import text is embedded in the module summary";
  return null;
}

assert(documentationIssue("//! opaque snapshots."));
assert(
  documentationIssue(
    "//! Opaque snapshots and PPM serialiuse crate::prelude::*;",
  ),
);
assert.equal(
  documentationIssue(
    "//! Opaque snapshots.\n//! ```\n//! use crate::Node;\n//! ```",
  ),
  null,
);

// Top-level items with their attributes, kind and line (0-based).
function topLevelItems(source) {
  const { tokens, pairs } = tokenize(source);
  const items = [];
  let attributes = [];
  let index = 0;
  while (index < tokens.length) {
    const token = tokens[index];
    if (token.kind === "comment") {
      index += 1;
      continue;
    }
    if (token.text === "#") {
      const inner = tokens[index + 1]?.text === "!";
      const open = index + (inner ? 2 : 1);
      const close = pairs.get(open);
      if (!inner)
        attributes.push(source.slice(tokens[open].start, tokens[close].end));
      index = close + 1;
      continue;
    }
    const start = index;
    let cursor = index;
    if (tokens[cursor].text === "pub") {
      cursor += 1;
      if (tokens[cursor]?.text === "(") cursor = pairs.get(cursor) + 1;
    }
    const modifier = (offset) =>
      ["unsafe", "async", "extern", "default"].includes(
        tokens[offset]?.text,
      ) ||
      (tokens[offset]?.text === "const" &&
        ["fn", "unsafe", "async", "extern"].includes(
          tokens[offset + 1]?.text,
        ));
    while (modifier(cursor)) {
      cursor += 1;
      if (tokens[cursor]?.kind === "literal") cursor += 1;
    }
    let kind = tokens[cursor]?.text;
    if (tokens[cursor + 1]?.text === "!") kind = "macro";
    const expression = ["const", "static", "type", "use", "macro"].includes(
      kind,
    );
    let end = cursor;
    while (end < tokens.length) {
      const text = tokens[end].text;
      if (text === ";") break;
      if ("([{".includes(text) && tokens[end].kind === "punctuation") {
        end = pairs.get(end);
        if (text === "{" && !expression) break;
        if (tokens[end + 1]?.text === ";" || text === "{") {
          if (tokens[end + 1]?.text === ";") end += 1;
          if (text === "{" && kind === "macro") break;
          if (tokens[end].text === ";") break;
        }
      }
      end += 1;
    }
    items.push({
      kind,
      line: tokens[start].line,
      attributes,
      test: attributes.some((value) => /^\[cfg\((?:all\()?test\b/.test(value)),
      declaration: kind === "mod" && tokens[end]?.text === ";",
    });
    attributes = [];
    index = end + 1;
  }
  return items;
}

function orderIssue(source) {
  const items = topLevelItems(source);
  let body = false;
  let tests = false;
  for (const item of items) {
    if (item.test) {
      // Out-of-line `mod tests;` may sit with the other declarations.
      if (item.kind === "mod" && !item.declaration) tests = true;
      continue;
    }
    if (tests)
      return `Line ${item.line + 1}: item follows the test module; keep tests last`;
    const header = item.kind === "use" || item.declaration;
    if (header && body)
      return `Line ${item.line + 1}: \`${item.kind}\` follows other items; move it to the file header`;
    if (!header) body = true;
  }
  return null;
}

assert.equal(
  orderIssue(
    "//! A.\nuse a::A;\nmod b;\npub use b::B;\nconst C: u8 = 1;\nfn f() {}\n#[cfg(test)]\nmod tests {}\n",
  ),
  null,
);
assert(orderIssue("fn f() {}\nmod late;\n"));
assert(orderIssue("fn f() {}\nuse late::Late;\n"));
assert(orderIssue("#[cfg(test)]\nmod tests {}\nfn f() {}\n"));
assert.equal(orderIssue("#[cfg(test)]\nmod tests;\nfn f() {}\n"), null);
assert.equal(
  orderIssue("const X: S = S { a: 1 };\nimpl S {}\nmacro_rules! m { () => {} }\nm!();\n"),
  null,
);
assert.equal(orderIssue("#[cfg(test)]\nfn helper() {}\nfn f() {}\n"), null);

function summaryIssue(relative, source) {
  if (isTestSource(relative) || relative.endsWith("/build.rs")) return null;
  const lines = source.split(/\r?\n/);
  if (!lines[0].startsWith("//!"))
    return "Missing `//!` module summary on the first line";
  const end = lines.findIndex((line) => !line.startsWith("//!"));
  if (end >= 0 && lines.slice(end).some((line) => line.startsWith("//!")))
    return "Module documentation must be one contiguous `//!` block";
  return null;
}

assert(summaryIssue("crates/a/src/b.rs", "use a::A;\n"));
assert(summaryIssue("crates/a/src/b.rs", "//! A.\n\n//! B.\n"));
assert.equal(summaryIssue("crates/a/src/b.rs", "//! A.\n//!\n//! B.\n"), null);
assert.equal(summaryIssue("crates/a/src/tests.rs", "use a::A;\n"), null);

function rustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory())
      return entry.name === "target" ? [] : rustFiles(file);
    return file.endsWith(".rs") ? [file] : [];
  });
}

function ownershipIssue(relative, source) {
  if (
    relative.startsWith("crates/motion-core/") &&
    /\b(?:struct|enum)\s+(?:CaptionCue|CaptionTrack|CueSheet|Scale)\b/.test(
      source,
    )
  )
    return "Diagram/caption tools belong in feature-selected motion-content modules";
  const prelude = source.split("pub mod prelude {")[1]?.split("\n}")[0] || "";
  if (
    /pub use [^;]*\b(?:diagram|captions|ink|Scale|LineChart|CaptionCue|CaptionTrack|BrushStroke|Ink|InkFill)\b[^;]*;/s.test(
      prelude,
    )
  )
    return "Optional production tools must remain namespaced, outside the prelude";
  return null;
}

assert(
  ownershipIssue(
    "crates/motion-core/src/caption.rs",
    "pub struct CaptionTrack;",
  ),
);
assert(
  ownershipIssue(
    "crates/motion/src/lib.rs",
    "pub mod prelude {\n pub use crate::Ink;\n}",
  ),
);
assert.equal(
  ownershipIssue(
    "crates/motion-content/src/captions/track.rs",
    "pub struct CaptionTrack;",
  ),
  null,
);

const failures = [];
for (const directory of ["crates", "gallery/src"]) {
  for (const file of rustFiles(path.join(root, directory))) {
    const relative = path.relative(root, file).split(path.sep).join("/");
    const source = fs.readFileSync(file, "utf8");
    const issue = documentationIssue(source);
    if (issue) failures.push(`${relative}: ${issue}`);
    const ownership = ownershipIssue(relative, source);
    if (ownership) failures.push(`${relative}: ${ownership}`);
    const organization = organizationIssue(relative, source);
    if (organization) failures.push(`${relative}: ${organization}`);
    const order = orderIssue(source);
    if (order) failures.push(`${relative}: ${order}`);
    const summary = summaryIssue(relative, source);
    if (summary) failures.push(`${relative}: ${summary}`);
    if (/^\s*use crate::prelude::/m.test(source))
      failures.push(`${relative}: Internal code imports the public prelude`);
    if (/^pub use motion_[a-z0-9_]+::\*;/m.test(source))
      failures.push(`${relative}: Upstream root glob reexport`);
  }
}

const metadata = spawnSync(
  "cargo",
  ["metadata", "--offline", "--no-deps", "--format-version", "1"],
  {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  },
);
assert.equal(metadata.status, 0, metadata.stderr || "Cargo metadata failed");
const packages = JSON.parse(metadata.stdout).packages;
const byName = new Map(packages.map((item) => [item.name, item]));
for (const [name, dependency] of [
  ["motion-core", "proptest"],
  ["motion-render", "proptest"],
  ["motion-bench", "criterion"],
]) {
  if (
    !byName
      .get(name)
      .dependencies.some(
        (item) => item.name === dependency && item.kind === "dev",
      )
  )
    failures.push(`${name}: Missing ${dependency} dev-dependency`);
}
if (
  !byName
    .get("motion-bench")
    .targets.some(
      (target) =>
        target.name === "criterion_frames" && target.kind.includes("bench"),
    )
)
  failures.push("motion-bench: Missing Criterion frame benchmark target");
const bytemuck = byName
  .get("motion-content")
  .dependencies.find((item) => item.name === "bytemuck");
if (!bytemuck?.features.includes("derive"))
  failures.push("motion-content: bytemuck must explicitly enable derive");
for (const [feature, prerequisite] of [
  ["diagram", "text"],
  ["captions", "text"],
  ["ink", "image"],
]) {
  if (!byName.get("motion-content").features[feature]?.includes(prerequisite))
    failures.push(
      `motion-content/${feature}: Missing ${prerequisite} prerequisite`,
    );
  if (
    !byName
      .get("motion-rs")
      .features[feature]?.includes(`motion-content/${feature}`)
  )
    failures.push(`motion-rs/${feature}: Missing content feature forwarding`);
}
// Applications built on the facade; every other workspace package sits below it.
const FACADE_CONSUMERS = ["motion-gallery", "motion-bench", "motion-cli"];
for (const item of packages) {
  if (!["motion-rs", ...FACADE_CONSUMERS].includes(item.name)) {
    if (
      item.dependencies.some((dependency) => dependency.name === "motion-rs")
    ) {
      failures.push(`${item.name}: Lower-level package depends on the facade`);
    }
  }
  if (["motion-rs", "motion-gallery", "motion-cli"].includes(item.name)) {
    if (
      item.targets.some(
        (target) =>
          target.kind.includes("custom-build") || target.kind.includes("bench"),
      )
    ) {
      failures.push(
        `${item.name}: Benchmark targets/build scripts must live in motion-bench`,
      );
    }
  }
}

for (const obsolete of [
  "native/ffmpeg/Cargo.toml",
  "benches/build.rs",
  "crates/motion/src/facade.rs",
  "crates/motion/src/lib.rs/core_integration_tests",
  "crates/motion-core/src/shapes.rs",
  "crates/motion-core/src/scene.rs",
  "crates/motion-content/src/outline_text.rs",
  "crates/motion-content/src/science.rs",
  "crates/motion-encode/src/production_api.rs",
  "crates/motion-encode/src/composition_output.rs",
  "crates/motion-encode/src/timeline_output.rs",
  "crates/motion-render/src/render.rs",
  "crates/motion-studio/src/worker.rs",
  "gallery/src/demos/mod.rs",
]) {
  if (fs.existsSync(path.join(root, obsolete)))
    failures.push(`${obsolete}: Obsolete source path remains`);
}

assert.equal(failures.length, 0, failures.join("\n"));
console.log(`Structure checks passed (${packages.length} packages).`);
