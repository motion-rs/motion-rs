const fs = require("node:fs");
const path = require("node:path");
const cp = require("node:child_process");
const assert = require("node:assert/strict");
const { loadReport, verifyComparable } = require("./report.cjs");
const ROOT = path.resolve(__dirname, "..");

function main() {
  const args = process.argv.slice(2),
    options = new Map();
  for (let index = 0; index < args.length; index += 2) {
    assert(
      ["--control", "--candidate", "--output", "--pairs", "--label"].includes(
        args[index],
      ) && args[index + 1],
      "Invalid pair arguments",
    );
    assert(!options.has(args[index]), "Repeated argument");
    options.set(args[index], args[index + 1]);
  }
  for (const required of ["--control", "--candidate", "--output", "--label"])
    assert(options.has(required), "Missing " + required);
  const count = Number(options.get("--pairs") || 6);
  assert(
    Number.isInteger(count) && count >= 2 && count <= 50,
    "Require 2..=50 pairs",
  );
  const control = loadReport(options.get("--control")),
    candidate = loadReport(options.get("--candidate"));
  verifyComparable(control, candidate);
  const directory = path.resolve(ROOT, options.get("--output"));
  fs.mkdirSync(directory, { recursive: true });
  assert(!fs.readdirSync(directory).length, "Pair output must be empty");
  const configuration = control.report.configuration,
    settings = control.report.settings;
  const env = {
    ...process.env,
    CARGO_INCREMENTAL: "0",
    MOTION_BENCH_WARMUPS: String(settings.warmups),
    MOTION_BENCH_SAMPLES: String(settings.samples),
    MOTION_BENCH_ITERATIONS: String(settings.iterations),
    MOTION_BENCH_MACHINE: control.report.environment.machine,
    MOTION_BENCH_CASES: configuration.cases.join(","),
    MOTION_BENCH_SIZE: configuration.size.join("x"),
    MOTION_BENCH_BACKENDS: configuration.backends
      .map((value) => (value === "Gpu" ? "gpu" : "cpu"))
      .join(","),
    MOTION_BENCH_COLD: configuration.cold ? "1" : "0",
    MOTION_BENCH_SCALING: configuration.scaling ? "1" : "0",
    MOTION_BENCH_SIZES: (configuration.sizes || [])
      .map((size) => size.join("x"))
      .join(",") || "1280x720,1920x1080",
    MOTION_BENCH_STAGES: configuration.stages ? "1" : "0",
    MOTION_BENCH_MICRO: configuration.micro ? "1" : "0",
    MOTION_BENCH_VIDEO: configuration.video ? "1" : "0",
    MOTION_BENCH_PARTICLES: String(configuration.particle_count || 72),
    MOTION_BENCH_VIDEO_FRAMES: String(configuration.video_frames || 8),
    MOTION_BENCH_SEQUENCES: configuration.sequence_frames ? "1" : "0",
    MOTION_BENCH_COMPARE: control.directory,
  };
  if (configuration.sequence_frames)
    env.MOTION_BENCH_SEQUENCE_FRAMES = String(configuration.sequence_frames);
  else delete env.MOTION_BENCH_SEQUENCE_FRAMES;
  const result = { label: options.get("--label"), pairs: [] };
  for (let index = 0; index < count; index++) {
    const pair = { order: index % 2 ? "candidate-first" : "control-first" };
    for (const role of index % 2
      ? ["candidate", "control"]
      : ["control", "candidate"]) {
      const current = role === "control" ? control : candidate;
      const output = path.join(
        directory,
        `${String(index + 1).padStart(2, "0")}-${role}`,
      );
      const run = cp.spawnSync(
        path.join(current.directory, "benchmark-executable"),
        [],
        {
          cwd: ROOT,
          encoding: "utf8",
          env: {
            ...env,
            MOTION_BENCH_OUTPUT: output,
            MOTION_BENCH_LABEL: result.label + "-" + role,
          },
          maxBuffer: 96 * 1024 * 1024,
        },
      );
      fs.writeFileSync(output + ".stdout.log", run.stdout || "");
      fs.writeFileSync(
        output + ".stderr.log",
        (run.stderr || "") + (run.error ? "\n" + run.error.message : ""),
      );
      assert.equal(
        run.status,
        0,
        `Pair run failed: ${role}, see ${output}.stderr.log`,
      );
      const relative = path.relative(ROOT, output).split(path.sep).join("/");
      verifyComparable(control, loadReport(relative));
      pair[role] = relative;
      console.log(`Pair ${index + 1}/${count} ${role}: verified`);
    }
    result.pairs.push(pair);
    fs.writeFileSync(
      path.join(directory, "step.json"),
      JSON.stringify(result, null, 2),
    );
  }
  console.log(
    "Verified paired run descriptor: " +
    path.join(directory, "step.json") +
    "\nCompare with: node benches/report.cjs --pairs " +
    path.relative(ROOT, directory),
  );
}

try {
  main();
} catch (error) {
  console.error("Paired run rejected: " + error.message);
  process.exitCode = 1;
}
