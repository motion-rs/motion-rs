// One-command performance comparison: builds and runs the `performance` bench for a control
// and the working tree, runs alternating AB/BA pairs and writes compare.md and compare.svg.
//
//   node benches/run.cjs [--control-ref <git-ref> | --control <run-dir>] [--pairs 6]
//                        [--features gpu,ffmpeg] [--phase warm-frame] [--label name]
//
// Any MOTION_BENCH_* variable (CASES, BACKENDS, SAMPLES, ...) is passed through.
const fs = require("node:fs");
const path = require("node:path");
const cp = require("node:child_process");
const assert = require("node:assert/strict");
const ROOT = path.resolve(__dirname, "..");

function run(command, args, options = {}) {
  console.log("$ " + [command, ...args].join(" "));
  const result = cp.spawnSync(command, args, { stdio: "inherit", ...options });
  assert.equal(result.status, 0, `${command} ${args[0]} failed`);
}

function bench(cwd, output, features, label) {
  const args = ["bench", "--locked", "-p", "motion-bench", "--bench", "performance"];
  if (features) args.push("--features", features);
  run("cargo", args, {
    cwd,
    env: {
      ...process.env,
      CARGO_INCREMENTAL: "0",
      MOTION_BENCH_OUTPUT: output,
      MOTION_BENCH_LABEL: label,
    },
  });
}

function main() {
  const args = process.argv.slice(2),
    options = new Map();
  for (let index = 0; index < args.length; index += 2) {
    assert(
      ["--control-ref", "--control", "--pairs", "--features", "--phase", "--label", "--output"].includes(args[index]) &&
      args[index + 1],
      "Usage: node benches/run.cjs [--control-ref <ref> | --control <run-dir>] [--pairs N] [--features f] [--phase p] [--label l] [--output dir]",
    );
    options.set(args[index], args[index + 1]);
  }
  assert(!(options.has("--control") && options.has("--control-ref")), "Choose one control source");
  const features = options.get("--features"),
    label = options.get("--label") || "run",
    out = path.resolve(
      ROOT,
      options.get("--output") || `target/performance/compare-${Date.now()}`,
    );
  fs.mkdirSync(out, { recursive: true });
  let control = options.get("--control") && path.resolve(ROOT, options.get("--control"));
  if (!control) {
    const ref = options.get("--control-ref") || "HEAD",
      source = path.join(out, "control-src");
    control = path.join(out, "control");
    run("git", ["worktree", "add", "--detach", source, ref], { cwd: ROOT });
    try {
      bench(source, control, features, label + "-control");
    } finally {
      cp.spawnSync("git", ["worktree", "remove", "--force", source], { cwd: ROOT, stdio: "inherit" });
    }
  }
  const candidate = path.join(out, "candidate");
  bench(ROOT, candidate, features, label + "-candidate");
  const pairs = path.join(out, "pairs");
  run("node", [
    path.join(__dirname, "pair.cjs"),
    "--control", control,
    "--candidate", candidate,
    "--output", pairs,
    "--label", label,
    "--pairs", options.get("--pairs") || "6",
  ], { cwd: ROOT });
  run("node", [path.join(__dirname, "report.cjs"), "--pairs", pairs, "--output", path.join(out, "compare.md")], { cwd: ROOT });
  run("node", [
    path.join(__dirname, "chart.cjs"),
    "--pairs", pairs,
    "--output", path.join(out, "compare.svg"),
    "--phase", options.get("--phase") || "warm-frame",
  ], { cwd: ROOT });
  console.log("\nReport: " + path.join(out, "compare.md") + "\nChart:  " + path.join(out, "compare.svg"));
}

try {
  main();
} catch (error) {
  console.error("Run failed: " + error.message);
  process.exitCode = 1;
}
