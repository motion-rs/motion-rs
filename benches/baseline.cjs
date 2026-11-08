const fs = require("node:fs");
const path = require("node:path");
const cp = require("node:child_process");
const assert = require("node:assert/strict");
const { loadReport, verifyComparable } = require("./report.cjs");

const ROOT = path.resolve(__dirname, "..");
const CASES = [
  "empty",
  "shapes",
  "svg",
  "text",
  "particles-orbit",
  "ink-long-stroke",
];

function groups() {
  return ["cpu", "gpu"].flatMap((backend) =>
    [...CASES, "export60"].map((name) => ({
      id: `${backend}-${name}`,
      backend,
      name,
      environment: {
        MOTION_BENCH_SIZE: "1920x1080",
        MOTION_BENCH_BACKENDS: backend,
        MOTION_BENCH_CASES: name === "export60" ? "empty" : name,
        MOTION_BENCH_PARTICLES: "10000",
        MOTION_BENCH_VIDEO_FRAMES: "60",
        MOTION_BENCH_VIDEO: name === "export60" ? "1" : "0",
        MOTION_BENCH_STAGES:
          backend === "cpu" && name !== "export60" ? "1" : "0",
        MOTION_BENCH_SEQUENCES: "0",
        MOTION_BENCH_MICRO: "0",
        MOTION_BENCH_COLD: "0",
        MOTION_BENCH_WARMUPS: "2",
        MOTION_BENCH_SAMPLES: name === "export60" ? "5" : "101",
        MOTION_BENCH_ITERATIONS: "1",
        MOTION_BENCH_FONT: path.join(
          ROOT,
          "tests/fixtures/fonts/NotoSans-Regular.ttf",
        ),
        MOTION_BENCH_MACHINE: `${process.platform}-${process.arch}-${require("node:os").cpus()[0].model}`,
      },
    })),
  );
}

function environment(group, directory) {
  const result = {
    ...process.env,
    ...group.environment,
    CARGO_INCREMENTAL: "0",
    MOTION_BENCH_OUTPUT: directory,
    MOTION_BENCH_LABEL: "fresh-baseline",
  };
  for (const key of Object.keys(result))
    if (
      key.startsWith("MOTION_BENCH_") &&
      !(key in group.environment) &&
      !["MOTION_BENCH_OUTPUT", "MOTION_BENCH_LABEL"].includes(key)
    )
      delete result[key];
  return result;
}

function execute(program, group, directory) {
  assert(
    !fs.existsSync(directory),
    "Run directory already exists: " + directory,
  );
  const started = Date.now();
  const run = cp.spawnSync(program, [], {
    cwd: ROOT,
    env: environment(group, directory),
    encoding: "utf8",
    maxBuffer: 96 * 1024 * 1024,
  });
  fs.writeFileSync(directory + ".stdout.log", run.stdout || "");
  fs.writeFileSync(
    directory + ".stderr.log",
    (run.stderr || "") + (run.error ? "\n" + run.error.message : ""),
  );
  let result = {
    path: path.relative(ROOT, directory),
    status: run.status,
    elapsed_seconds: (Date.now() - started) / 1000,
  };
  if (run.status === 0) {
    const verified = loadReport(result.path);
    result.records = verified.report.records.length;
    result.executable = verified.report.provenance.executable_fingerprint;
  } else {
    result.error = (
      run.stderr ||
      run.stdout ||
      run.error?.message ||
      "Unknown benchmark failure"
    )
      .trim()
      .split("\n")
      .at(-1);
  }
  console.log(
    `${group.id}: ${result.status === 0 ? "verified" : "FAILED"} (${result.elapsed_seconds.toFixed(1)}s) ${result.error || ""}`,
  );
  return result;
}

function save(file, state) {
  fs.writeFileSync(file, JSON.stringify(state, null, 2) + "\n");
}

function register(state) {
  const file = path.join(ROOT, "docs/performance-runs.json");
  const history = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.equal(history.schema, "motion-performance-history-v1");
  const statePath = path.relative(ROOT, state.file);
  assert.notEqual(
    history.baseline?.state,
    statePath,
    "Baseline already registered",
  );
  const ids = new Set(state.groups.map((group) => "fresh-1080p-" + group.id));
  const previous = history.series.filter((series) => ids.has(series.id));
  if (previous.length) {
    const archive = path.join(state.directory, "prior-performance-runs.json");
    const report = path.join(state.directory, "prior-performance.md");
    if (fs.existsSync(archive)) {
      assert(
        fs.readFileSync(archive).equals(fs.readFileSync(file)),
        "Prior baseline archive differs",
      );
    } else {
      fs.copyFileSync(file, archive);
      fs.copyFileSync(path.join(ROOT, "docs/benchmarks.md"), report);
    }
    history.baseline_archives ??= [];
    history.baseline_archives.push({
      manifest: path.relative(ROOT, archive),
      report: path.relative(ROOT, report),
      baseline: history.baseline,
      series: previous.length,
    });
    history.series = history.series.filter((series) => !ids.has(series.id));
  }
  const failures = state.groups
    .filter((group) => group.probe.status !== 0)
    .map((group) => `${group.id}: ${group.probe.error}`)
    .join("; ");
  for (const group of state.groups) {
    if (group.probe.status !== 0) continue;
    const runs = [group.probe, ...(group.repeats || [])];
    assert(
      runs.length === 3 && runs.every((run) => run.status === 0),
      "Require three healthy independent executions",
    );
    const id = "fresh-1080p-" + group.id;
    assert(
      !history.series.some((series) => series.id === id),
      "Baseline already registered: " + id,
    );
    const first = loadReport(runs[0].path);
    for (const run of runs) verifyComparable(first, loadReport(run.path));
    history.series.push({
      id,
      title: `Fresh Baseline / ${group.id} / 1920x1080`,
      steps: [
        {
          label: "00 three independent runs",
          change:
            "Measurement-only baseline of frozen executable " +
            group.probe.executable +
            ". No engine optimization or quota change.",
          verification:
            "Three independent executions; exact reference pixels, checksums, configuration and embedded executable/source provenance validated. Phase timings remain separate. Raw state: " +
            path.relative(ROOT, state.file) +
            ".",
          notes:
            "Final quality, existing 1 GiB/20-billion work benchmark limits, not default authoring limits. 101 individual frame/operation samples support observed P95; five full-export samples do not. No optimization, GUI FPS, RSS or causal speedup claim. Rejected cohorts remain failures: " +
            (failures || "none") +
            ".",
          runs: runs.map((run) => run.path),
        },
      ],
    });
  }
  history.baseline = {
    state: statePath,
    source: state.source,
    failures: state.groups
      .filter((group) => group.probe.status !== 0)
      .map((group) => ({ id: group.id, ...group.probe })),
  };
  save(file, history);
}

function resetHistory(root = ROOT) {
  root = fs.realpathSync(root);
  const manifest = path.join(root, "docs/performance-runs.json");
  const previous = fs.readFileSync(manifest);
  const history = JSON.parse(previous);
  assert.equal(history.schema, "motion-performance-history-v1");
  const directories = [
    "target/performance",
    "crates/motion/target/performance",
  ];
  for (const relative of directories) {
    const directory = path.join(root, relative);
    assert(
      !cp
        .execFileSync("git", ["ls-files", "--", relative], {
          cwd: root,
          encoding: "utf8",
        })
        .trim(),
      "Refuse to delete tracked files",
    );
    if (fs.existsSync(directory))
      assert.equal(
        fs.realpathSync(directory),
        directory,
        "Refuse a redirected history root",
      );
  }
  const reset = {
    source_commit: cp
      .execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: root,
        encoding: "utf8",
      })
      .trim(),
    previous_manifest_sha256: require("node:crypto")
      .createHash("sha256")
      .update(previous)
      .digest("hex"),
    previous_series: history.series.length,
    cleared_directories: directories,
    interpretation:
      "Historical measurements and plan archives removed from the active baseline. Tracked history remains recoverable from Git; ignored raw artifacts have been deleted.",
  };
  for (const relative of directories)
    fs.rmSync(path.join(root, relative), { recursive: true, force: true });
  save(manifest, { schema: history.schema, series: [], reset });
  console.log(
    "Cleared historical performance artifacts and reset active manifest",
  );
}

async function profile(program, group, directory) {
  assert(!fs.existsSync(directory), "Profile output already exists");
  const stdout = fs.openSync(directory + ".stdout.log", "wx");
  const stderr = fs.openSync(directory + ".stderr.log", "wx");
  const child = cp.spawn(program, [], {
    cwd: ROOT,
    env: environment(group, directory),
    stdio: ["ignore", stdout, stderr],
  });
  const completed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  let sample;
  try {
    sample = cp.spawnSync(
      "/usr/bin/sample",
      [
        String(child.pid),
        "30",
        "1",
        "-mayDie",
        "-file",
        directory + ".sample.txt",
      ],
      { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 },
    );
    fs.writeFileSync(
      directory + ".sampler.log",
      (sample.stdout || "") +
        (sample.stderr || "") +
        (sample.error?.message || ""),
    );
    const result = await completed;
    return {
      ...result,
      sample_status: sample.status,
      path: path.relative(ROOT, directory),
      sample: path.relative(ROOT, directory + ".sample.txt"),
      included_in_timings: false,
    };
  } finally {
    fs.closeSync(stdout);
    fs.closeSync(stderr);
  }
}

async function main() {
  if (process.argv.includes("--self-test")) {
    const plan = groups();
    assert.equal(plan.length, 14);
    assert.equal(new Set(plan.map((group) => group.id)).size, 14);
    assert.equal(plan.filter((group) => group.name === "export60").length, 2);
    for (const group of plan) {
      assert.equal(group.environment.MOTION_BENCH_SIZE, "1920x1080");
      assert.equal(group.environment.MOTION_BENCH_PARTICLES, "10000");
      assert.equal(
        group.environment.MOTION_BENCH_SAMPLES,
        group.name === "export60" ? "5" : "101",
      );
      assert(!("MOTION_BENCH_LIMIT" in group.environment));
    }
    const env = environment(plan[0], "/tmp/example");
    assert.equal(env.MOTION_BENCH_BACKENDS, "cpu");
    assert(!("MOTION_BENCH_COMPARE" in env));
    process.env.MOTION_BENCH_SEQUENCE_FRAMES = "7";
    assert(
      !("MOTION_BENCH_SEQUENCE_FRAMES" in environment(plan[0], "/tmp/example")),
    );
    delete process.env.MOTION_BENCH_SEQUENCE_FRAMES;
    const temporary = fs.realpathSync(
      fs.mkdtempSync(
        path.join(require("node:os").tmpdir(), "motion-baseline-reset-"),
      ),
    );
    const original = cp.execFileSync;
    let tracked = true;
    try {
      for (const folder of [
        "docs",
        "target/performance",
        "crates/motion/target/performance",
        "target/debug",
      ])
        fs.mkdirSync(path.join(temporary, folder), { recursive: true });
      save(path.join(temporary, "docs/performance-runs.json"), {
        schema: "motion-performance-history-v1",
        series: [{ id: "old" }],
        plan_archives: ["old"],
      });
      for (const filename of [
        "target/performance/raw",
        "crates/motion/target/performance/raw",
        "target/debug/keep",
        "draft",
      ])
        fs.writeFileSync(path.join(temporary, filename), "fixture");
      cp.execFileSync = (program, args, options) =>
        options.cwd === temporary
          ? args[0] === "ls-files"
            ? tracked
              ? "protected\n"
              : ""
            : "fixture-commit\n"
          : original(program, args, options);
      assert.throws(() => resetHistory(temporary), /tracked files/);
      assert(fs.existsSync(path.join(temporary, "target/performance/raw")));
      tracked = false;
      resetHistory(temporary);
      assert(
        !fs.existsSync(path.join(temporary, "target/performance")) &&
          !fs.existsSync(
            path.join(temporary, "crates/motion/target/performance"),
          ),
      );
      assert(
        fs.existsSync(path.join(temporary, "target/debug/keep")) &&
          fs.existsSync(path.join(temporary, "draft")),
      );
      const reset = JSON.parse(
        fs.readFileSync(
          path.join(temporary, "docs/performance-runs.json"),
          "utf8",
        ),
      );
      assert.equal(reset.reset.previous_series, 1);
      assert.deepEqual(reset.series, []);
      assert(!("plan_archives" in reset));
      fs.symlinkSync(
        path.join(temporary, "target/debug"),
        path.join(temporary, "target/performance"),
      );
      assert.throws(() => resetHistory(temporary), /redirected history root/);
      assert(fs.existsSync(path.join(temporary, "target/debug/keep")));
    } finally {
      cp.execFileSync = original;
      fs.rmSync(temporary, { recursive: true, force: true });
    }
    console.log(
      "PASS: 14 fixed 1080p cohorts, sufficient frame samples, 10000 particles, 60-frame exports and isolated environment",
    );
    console.log(
      "PASS: reset refuses tracked/symlink roots and preserves build caches and drafts",
    );
    return;
  }
  const args = process.argv.slice(2),
    options = new Map();
  for (let index = 0; index < args.length; index += 2) {
    assert(
      ["--phase", "--state", "--program"].includes(args[index]) &&
        args[index + 1],
      "Invalid baseline arguments",
    );
    assert(!options.has(args[index]), "Duplicate baseline argument");
    options.set(args[index], args[index + 1]);
  }
  const phase = options.get("--phase");
  assert(
    ["reset", "probes", "repeats", "profile", "register"].includes(phase),
    "Require --phase reset|probes|repeats|profile|register",
  );
  if (phase === "reset") return resetHistory();
  assert(options.has("--state"), "Require --state");
  const file = path.resolve(options.get("--state"));
  let state;
  if (phase === "probes") {
    assert(
      !fs.existsSync(file),
      "Baseline state is immutable; choose a fresh path",
    );
    assert(options.has("--program"), "Probes require --program");
    const directory = file.replace(/\.json$/, "") + "-runs";
    fs.mkdirSync(directory, { recursive: true });
    assert(!fs.readdirSync(directory).length, "Baseline output must be empty");
    const diff = cp.execFileSync(
      "git",
      [
        "diff",
        "--binary",
        "HEAD",
        "--",
        "crates",
        "benches",
        "Cargo.toml",
        "Cargo.lock",
      ],
      { cwd: ROOT, maxBuffer: 32 * 1024 * 1024 },
    );
    state = {
      schema: "motion-fresh-baseline-v1",
      file,
      program: path.resolve(options.get("--program")),
      directory,
      groups: groups(),
      source: {
        commit: cp
          .execFileSync("git", ["rev-parse", "HEAD"], {
            cwd: ROOT,
            encoding: "utf8",
          })
          .trim(),
        working_diff_sha256: require("node:crypto")
          .createHash("sha256")
          .update(diff)
          .digest("hex"),
      },
      interpretation:
        "Three independent executions of a frozen executable. No optimization or causal speedup claim. 101 individual frame samples; five whole-export samples are insufficient for P95.",
    };
    fs.writeFileSync(path.join(directory, "source.diff"), diff);
    save(file, state);
    for (const group of state.groups) {
      group.probe = execute(
        state.program,
        group,
        path.join(directory, group.id + "-probe"),
      );
      save(file, state);
    }
  } else {
    state = JSON.parse(fs.readFileSync(file, "utf8"));
    assert.equal(state.schema, "motion-fresh-baseline-v1");
    if (phase === "register") {
      register(state);
    } else if (phase === "repeats") {
      for (const group of state.groups) {
        if (group.probe?.status !== 0) continue;
        const probe = loadReport(group.probe.path);
        const program = path.join(probe.directory, "benchmark-executable");
        group.repeats ??= [];
        for (let index = group.repeats.length; index < 2; index++) {
          const result = execute(
            program,
            group,
            path.join(state.directory, `${group.id}-repeat-${index + 1}`),
          );
          group.repeats.push(result);
          save(file, state);
          assert.equal(
            result.status,
            0,
            "Repeated baseline failed; inspect retained logs",
          );
          verifyComparable(probe, loadReport(result.path));
        }
      }
    } else {
      const group = state.groups.find((group) => group.id === "cpu-export60");
      assert.equal(
        group?.probe?.status,
        0,
        "A complete CPU export probe must pass before profiling",
      );
      const program = path.join(
        loadReport(group.probe.path).directory,
        "benchmark-executable",
      );
      state.profile = await profile(
        program,
        group,
        path.join(state.directory, "cpu-export60-profile"),
      );
      save(file, state);
      console.log("Profile result: " + JSON.stringify(state.profile));
    }
  }
}

if (require.main === module)
  main().catch((error) => {
    console.error(error.stack);
    process.exitCode = 1;
  });
