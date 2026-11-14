const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");

const ROOT = path.resolve(__dirname, "..");
const MANIFEST_SCHEMA = "motion-performance-history-v1";
const BENCHMARKS_DOCUMENT = path.join(ROOT, "docs/en/benchmarks.md");
const REPORT_START = "<!-- performance-report:start -->";
const REPORT_END = "<!-- performance-report:end -->";

function stable(value) {
  if (Array.isArray(value)) return "[" + value.map(stable).join(",") + "]";
  if (value && typeof value === "object") {
    return (
      "{" +
      Object.keys(value)
        .sort()
        .map((key) => JSON.stringify(key) + ":" + stable(value[key]))
        .join(",") +
      "}"
    );
  }
  return JSON.stringify(value);
}

function median(values) {
  const sorted = [...values].sort((first, second) => first - second);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
}

function improvement(previous, current) {
  return previous > 0 ? ((previous - current) / previous) * 100 : null;
}

function percentage(value) {
  return value === null
    ? "N/A"
    : (value > 0 ? "+" : "") + value.toFixed(2) + "%";
}

function escape(value) {
  return String(value)
    .replaceAll("|", "\\|")
    .replace(/[\r\n]/g, " ");
}

function fingerprint(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes)
    hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  return hash.toString(16).padStart(16, "0");
}

function foldChecksum(values) {
  let state = 0n;
  for (const value of values)
    state = BigInt.asUintN(
      64,
      ((state << 7n) | (state >> 57n)) +
        BigInt("0x" + value) +
        0x9e3779b97f4a7c15n,
    );
  return state.toString(16).padStart(16, "0");
}

function configurationKey(report) {
  return stable({
    schema: report.schema,
    suite: report.suite,
    settings: report.settings,
    configuration: report.configuration,
    environment: report.environment,
  });
}

function validate(report) {
  assert.equal(report.schema, "motion-bench-v3", "Unsupported report schema");
  assert(report.records.length > 0, "Empty report");
  assert(
    report.references && Object.keys(report.references).length > 0,
    "Missing reference images",
  );
  assert(
    /^\d+$/.test(report.execution?.started_unix_ns) &&
      /^\d+$/.test(report.execution?.finished_unix_ns),
    "Missing execution interval",
  );
  assert(
    BigInt(report.execution.started_unix_ns) <=
      BigInt(report.execution.finished_unix_ns),
    "Invalid execution interval",
  );
  const names = new Set();
  for (const record of report.records) {
    assert(
      record.metrics && typeof record.metrics === "object",
      "Missing measurement metrics",
    );
    assert(!names.has(record.name), "Duplicate record: " + record.name);
    names.add(record.name);
    const measurement = record.measurement;
    const sequence = measurement.timing_kind === "frame-sequence";
    if (sequence)
      assert(
        Number.isSafeInteger(report.configuration?.sequence_frames) &&
          report.configuration.sequence_frames >= 2,
        "Missing frame schedule",
      );
    assert(
      [
        "frame-sequence",
        "single-operation",
        "fresh-input-operation",
        "batch-mean",
        "mean-of-individual-operations",
      ].includes(measurement.timing_kind),
      "Missing timing semantics",
    );
    assert.equal(
      measurement.samples.length,
      sequence ? report.configuration.sequence_frames : report.settings.samples,
      "Sample count changed",
    );
    assert(/^[0-9a-f]{16}$/.test(measurement.checksum), "Invalid checksum");
    const values = measurement.samples
      .map((sample) => {
        assert(
          Number.isSafeInteger(sample.elapsed_ns) && sample.elapsed_ns >= 0,
          "Invalid raw duration",
        );
        assert(
          Number.isSafeInteger(sample.iterations) && sample.iterations > 0,
          "Invalid iteration count",
        );
        assert(
          /^[0-9a-f]{16}$/.test(sample.checksum),
          "Invalid sample checksum",
        );
        if (sequence) {
          assert(
            Number.isSafeInteger(sample.frame) && sample.frame >= 0,
            "Missing measured frame",
          );
          assert.equal(
            sample.iterations,
            1,
            "Frame latency cannot be a batch mean",
          );
        } else assert.equal(sample.frame, null, "Unexpected frame metadata");
        return sample.elapsed_ns / sample.iterations / 1000;
      })
      .sort((first, second) => first - second);
    assert.equal(
      measurement.checksum,
      foldChecksum(measurement.samples.map((sample) => sample.checksum)),
      "Aggregate checksum disagrees with raw samples",
    );
    if (sequence)
      assert.deepEqual(
        measurement.samples
          .map((sample) => sample.frame)
          .sort((first, second) => first - second),
        Array.from(
          { length: report.configuration.sequence_frames },
          (_, index) => index,
        ),
        "Frame schedule must be a complete deterministic permutation",
      );
    for (const [field, expected] of [
      ["p50_us", values[Math.ceil(values.length * 0.5) - 1]],
      ["p95_us", values[Math.ceil(values.length * 0.95) - 1]],
      ["min_us", values[0]],
      ["max_us", values.at(-1)],
    ]) {
      assert(
        Number.isFinite(measurement[field]) && measurement[field] >= 0,
        "Invalid " + field,
      );
      assert(
        Math.abs(measurement[field] - expected) <=
          Math.max(1e-9, expected * 1e-12),
        "Summary disagrees with raw samples",
      );
    }
  }
}

function loadReport(relative) {
  const directory = path.resolve(ROOT, relative);
  const report = JSON.parse(
    fs.readFileSync(path.join(directory, "report.json"), "utf8"),
  );
  validate(report);
  assert.equal(
    report.provenance.executable,
    "benchmark-executable",
    "Missing frozen executable",
  );
  assert.equal(
    report.provenance.source_capture,
    "compile-time",
    "Source provenance must be embedded at build time",
  );
  assert.equal(
    fingerprint(fs.readFileSync(path.join(directory, "benchmark-executable"))),
    report.provenance.executable_fingerprint,
    "Frozen executable changed",
  );
  for (const [name, expected] of Object.entries(report.references)) {
    assert(
      !name.includes("/") && !name.includes(".."),
      "Unsafe reference name",
    );
    assert.equal(
      fingerprint(fs.readFileSync(path.join(directory, name))),
      expected,
      "Reference file changed: " + name,
    );
  }
  return { report, directory, relative };
}

function verifyComparable(first, candidate) {
  assert.equal(
    configurationKey(candidate.report),
    configurationKey(first.report),
    "Incompatible benchmark configuration",
  );
  assert.deepEqual(
    Object.keys(candidate.report.references).sort(),
    Object.keys(first.report.references).sort(),
    "Different reference sets",
  );
  for (const name of Object.keys(first.report.references)) {
    assert(
      fs
        .readFileSync(path.join(first.directory, name))
        .equals(fs.readFileSync(path.join(candidate.directory, name))),
      "Pixels changed: " + name,
    );
  }
  const original = new Map(
    first.report.records.map((record) => [record.name, record]),
  );
  assert.equal(
    candidate.report.records.length,
    original.size,
    "Different measurement sets",
  );
  for (const record of candidate.report.records) {
    const previous = original.get(record.name);
    assert(previous, "Unknown measurement: " + record.name);
    assert.equal(
      record.measurement.checksum,
      previous.measurement.checksum,
      "Checksum changed: " + record.name,
    );
    assert.equal(
      stable(record.metrics.adapter),
      stable(previous.metrics.adapter),
      "Adapter changed: " + record.name,
    );
  }
}

function estimate(values) {
  assert(
    values.length > 0 && values.every(Number.isFinite),
    "Invalid paired changes",
  );
  const center = median(values);
  if (values.length < 5)
    return { center, low: null, high: null, status: "exploratory (<5 pairs)" };
  let state = 0x5eed1234;
  const random = () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 4294967296;
  };
  const bootstrap = Array.from({ length: 2000 }, () =>
    median(
      Array.from(
        { length: values.length },
        () => values[Math.floor(random() * values.length)],
      ),
    ),
  ).sort((first, second) => first - second);
  const low = bootstrap[Math.floor(bootstrap.length * 0.025)],
    high = bootstrap[Math.ceil(bootstrap.length * 0.975) - 1];
  return {
    center,
    low,
    high,
    status: low > 0 ? "improved" : high < 0 ? "regressed" : "uncertain",
  };
}

function interval(result) {
  if (result.center === null) return "N/A (zero control)";
  return (
    percentage(result.center) +
    (result.low === null
      ? " (exploratory)"
      : ` [${percentage(result.low)}, ${percentage(result.high)}]`)
  );
}

function pairedEstimate(previous, current) {
  const changes = current.map((value, index) =>
    improvement(previous[index], value),
  );
  return changes.every(Number.isFinite)
    ? estimate(changes)
    : {
        center: null,
        low: null,
        high: null,
        status: "unmeasurable (zero control)",
      };
}

function spread(values) {
  const center = median(values),
    deviation = median(values.map((value) => Math.abs(value - center)));
  return center > 0 ? percentage((deviation / center) * 100) : "N/A";
}

function pairedRuns(step, previous, first, load) {
  assert(
    Array.isArray(step.pairs) && step.pairs.length > 0,
    "Optimization steps require fresh control/candidate pairs",
  );
  const seen = new Set(),
    candidates = [],
    controls = [];
  const expectedOrder = ["control-first", "candidate-first"];
  let previousEnd = null;
  for (const [index, pair] of step.pairs.entries()) {
    assert.equal(
      pair.order,
      expectedOrder[index % 2],
      "Pairs must alternate control/candidate execution order",
    );
    assert(
      !seen.has(pair.control) &&
        !seen.has(pair.candidate) &&
        pair.control !== pair.candidate,
      "Pair runs must be unique",
    );
    seen.add(pair.control);
    seen.add(pair.candidate);
    const control = load(pair.control),
      candidate = load(pair.candidate);
    verifyComparable(first, control);
    verifyComparable(first, candidate);
    assert.equal(
      control.report.provenance.executable_fingerprint,
      previous[0].report.provenance.executable_fingerprint,
      "Control is not the previous-step executable",
    );
    if (candidates.length)
      assert.equal(
        candidate.report.provenance.executable_fingerprint,
        candidates[0].report.provenance.executable_fingerprint,
        "Candidate executable changed within a step",
      );
    const ordered =
      pair.order === "control-first"
        ? [control, candidate]
        : [candidate, control];
    const start = BigInt(ordered[0].report.execution.started_unix_ns),
      middleEnd = BigInt(ordered[0].report.execution.finished_unix_ns),
      middleStart = BigInt(ordered[1].report.execution.started_unix_ns),
      end = BigInt(ordered[1].report.execution.finished_unix_ns);
    assert(
      middleEnd <= middleStart &&
        (previousEnd === null || previousEnd <= start),
      "Runs overlap or do not follow the recorded paired order",
    );
    previousEnd = end;
    controls.push(control);
    candidates.push(candidate);
  }
  return { candidates, controls };
}

function document(manifest, load = loadReport, details = false) {
  assert.equal(
    manifest.schema,
    MANIFEST_SCHEMA,
    "Invalid history manifest schema",
  );
  const lines = [
    "# Performance",
    "",
    manifest.measurement
      ? "These are current-source non-film benchmark observations, not an optimization against deleted historical measurements."
      : manifest.baseline
        ? "This is the active performance baseline. It measures the frozen source identified below, not an optimization against historical binaries."
        : "This is the concise performance report. Historical measurements describe frozen executables, not a fresh measurement of current HEAD.",
    "Registered measurements, verification notes and raw-run paths are in [performance-runs.json](performance-runs.json). Raw runs retain JSON/TSV, samples and reference images.",
    "",
    "## Interpretation",
    "",
    "- Time improvement = `(reference - current) / reference * 100%`. Positive is faster; negative is a regression.",
    '- "Previous" compares with the immediately preceding step in the same series. "Initial" always compares with step 00.',
    "- Values are medians across independent runs; MAD% reports run-to-run dispersion. Batch P95 is never called frame-tail latency.",
    "- Previous-step improvement uses fresh paired controls with alternating A/B then B/A order. Historical baseline comparison is descriptive, not a causal estimate.",
    "- Paired P50 changes have a deterministic bootstrap interval after at least five pairs. Smaller cohorts are exploratory; an interval crossing zero is uncertain.",
    "- P95 requires at least 100 individual frame/single-operation samples in every run; otherwise it is explicitly insufficient.",
    "- Cases, resolution, quality, budgets, fonts, features, sampling settings, machine/compiler and adapter must match within a series.",
    "- Checksums, exact reference pixels and raw sample summaries are validated. Logical peak bytes/work are not process RSS or wall-clock time.",
    "- No cross-case average speedup is presented. Cold sessions, warm frames, seeks, I/O and encode timings stay separate.",
    "",
  ];
  if (manifest.measurement) {
    const measurement = manifest.measurement;
    lines.push(
      "## Current Measurement",
      "",
      `Source commit: ${escape(measurement.source_commit)}.`,
      "",
      `Targets: ${measurement.targets.map(escape).join(", ")}. Excluded: ${measurement.excluded_targets.map(escape).join(", ")}.`,
      "",
      escape(measurement.sampling),
      "",
      escape(measurement.scope),
      "",
      `Historical series removed: ${manifest.reset?.previous_series ?? "not recorded"}.`,
      "",
    );
    if (measurement.failures?.length) {
      lines.push("### Incomplete Targets", "");
      for (const failure of measurement.failures)
        lines.push(`- ${escape(failure.target)}: ${escape(failure.error)}`);
      lines.push("");
    }
    if (measurement.criterion) {
      lines.push(
        "### Criterion CPU Warm Frames",
        "",
        "Criterion estimates come from batches of iterations, not individual-frame P95 or independent-run speedup evidence.",
        "",
        "| Case | Median Estimate (us) | 95% Interval (us) | Samples |",
        "| --- | ---: | --- | ---: |",
      );
      for (const record of measurement.criterion.records)
        lines.push(
          `| ${escape(record.name)} | ${(record.median_ns / 1000).toFixed(3)} | ${(record.lower_ns / 1000).toFixed(3)} .. ${(record.upper_ns / 1000).toFixed(3)} | ${record.samples} |`,
        );
      lines.push(
        "",
        `Raw Criterion data: \`${escape(measurement.criterion.directory)}\`.`,
        "",
      );
    }
  }
  if (manifest.baseline) {
    lines.push(
      "## Active Baseline",
      "",
      `Source commit: ${manifest.baseline.source.commit}; uncommitted code diff SHA-256: ${manifest.baseline.source.working_diff_sha256}.`,
      "",
      "Three independent runs per healthy cohort, Final quality, 1920x1080, fixed Noto Sans, 10000 particles and existing 1 GiB/20-billion-work benchmark limits. These are not default authoring limits. Frame/operation cohorts use 101 samples after two warmups; 60-frame exports use five samples.",
      "",
      `Historical series cleared: ${manifest.reset?.previous_series ?? "not recorded"}. Rejected cohorts: ${manifest.baseline.failures.length ? manifest.baseline.failures.map((failure) => escape(failure.id + ": " + failure.error)).join("; ") : "none"}.`,
      "",
      "P50/P95 below are absolute observations. This is not Studio presentation FPS, RSS, a full-film gate or evidence of improvement over the deleted history.",
      "",
    );
  }
  if (!details)
    lines.push(
      "## Conclusions",
      "",
      "One row per series, using its latest step. Outcomes count case/phase comparisons, not a cross-case average. Representative deltas show up to two largest absolute P50 changes; every other result and negative control remains available in the detailed report.",
      "",
      "| Series / Latest Step | Runs | Cases | P50 Outcomes | P95 Eligible Cases | Representative Timing / P50 Change |",
      "| --- | ---: | ---: | --- | ---: | --- |",
    );
  if (!manifest.series.length)
    lines.push(
      "No baseline has been registered. No optimization result is claimed.",
      "",
    );
  const ids = new Set();
  for (const series of manifest.series) {
    assert(!ids.has(series.id), "Duplicate series");
    ids.add(series.id);
    assert(series.steps.length > 0, "Series requires an initial baseline");
    if (details)
      lines.push(
        "## " + escape(series.title),
        "",
        "| Step | Case / Phase | Runs | P50 us | MAD% | P95 us | P50 vs Previous (95% interval) | P50 vs Initial | Assessment | P95 vs Previous | P95 vs Initial | Peak Logical MiB | Work Units |",
        "| --- | --- | ---: | ---: | ---: | --- | --- | --- | --- | --- | --- | ---: | ---: |",
      );
    let first, previousRuns, initialRuns, latestRuns;
    const latest = [];
    const labels = new Set();
    for (const [stepIndex, step] of series.steps.entries()) {
      assert(
        step.label && !labels.has(step.label),
        "Missing or duplicate step label",
      );
      labels.add(step.label);
      let runs,
        controls = [];
      if (stepIndex === 0) {
        assert(
          Array.isArray(step.runs) && step.runs.length > 0,
          "Initial step requires raw runs",
        );
        assert.equal(
          new Set(step.runs).size,
          step.runs.length,
          "Repeated run is not independent",
        );
        runs = step.runs.map(load);
      } else {
        assert(
          [step.change, step.verification, step.notes].every(
            (value) =>
              typeof value === "string" &&
              value.trim() &&
              !value.startsWith("REQUIRED:"),
          ),
          "Optimization steps require completed change, verification and regression notes",
        );
        ({ candidates: runs, controls } = pairedRuns(
          step,
          previousRuns,
          first,
          load,
        ));
      }
      first ??= runs[0];
      for (const run of runs) verifyComparable(first, run);
      initialRuns ??= runs;
      if (runs.length > 1)
        for (const run of runs)
          assert.equal(
            run.report.provenance.executable_fingerprint,
            runs[0].report.provenance.executable_fingerprint,
            "A step must use one executable",
          );
      for (const original of first.report.records) {
        const records = runs.map((run) =>
          run.report.records.find((record) => record.name === original.name),
        );
        const values = records.map((record) => record.measurement.p50_us);
        const p50 = median(values),
          p95 = median(records.map((record) => record.measurement.p95_us));
        const initialRecords = initialRuns.map((run) =>
          run.report.records.find((record) => record.name === original.name),
        );
        const controlRecords = controls.map((run) =>
          run.report.records.find((record) => record.name === original.name),
        );
        const paired = stepIndex
          ? pairedEstimate(
              controlRecords.map((record) => record.measurement.p50_us),
              records.map((record) => record.measurement.p50_us),
            )
          : null;
        const tailEnough = records
          .concat(initialRecords, controlRecords)
          .every(
            (record) =>
              !["batch-mean", "mean-of-individual-operations"].includes(
                record.measurement.timing_kind,
              ) &&
              record.measurement.samples.length >= 100 &&
              record.measurement.samples.every(
                (sample) => sample.iterations === 1,
              ),
          );
        const tail = tailEnough
          ? p95.toFixed(3)
          : "insufficient / " + p95.toFixed(3) + " observed";
        const tailPrevious =
          stepIndex && tailEnough
            ? interval(
                pairedEstimate(
                  controlRecords.map((record) => record.measurement.p95_us),
                  records.map((record) => record.measurement.p95_us),
                ),
              )
            : stepIndex
              ? "insufficient"
              : "baseline";
        const tailInitial =
          stepIndex && tailEnough
            ? percentage(
                improvement(
                  median(
                    initialRecords.map((record) => record.measurement.p95_us),
                  ),
                  p95,
                ),
              )
            : stepIndex
              ? "insufficient"
              : "baseline";
        const peak = records
          .map((record) => record.metrics.peak_bytes)
          .filter(Number.isFinite);
        const work = records
          .map((record) => record.metrics.work)
          .filter(Number.isFinite);
        const initial = stepIndex
          ? percentage(
              improvement(
                median(
                  initialRecords.map((record) => record.measurement.p50_us),
                ),
                p50,
              ),
            )
          : "baseline";
        if (details)
          lines.push(
            `| ${escape(step.label)} | ${escape(original.name)} | ${runs.length} | ${p50.toFixed(3)} | ${spread(values)} | ${tail} | ${paired ? interval(paired) : "baseline"} | ${initial} | ${paired ? paired.status : "initial baseline"} | ${tailPrevious} | ${tailInitial} | ${peak.length ? (Math.max(...peak) / 1048576).toFixed(3) : "N/A"} | ${work.length ? Math.max(...work) : "N/A"} |`,
          );
        if (stepIndex === series.steps.length - 1)
          latest.push({
            name: original.name,
            paired,
            initial,
            tailEnough,
            p50,
            p95,
          });
      }
      previousRuns = runs;
      latestRuns = runs.length;
    }
    if (!details) {
      const outcomes = new Map();
      for (const record of latest) {
        const status = record.paired?.status || "baseline (no speedup claim)";
        outcomes.set(status, (outcomes.get(status) || 0) + 1);
      }
      const representative =
        latest
          .filter((record) => record.paired && record.paired.center !== null)
          .sort(
            (first, second) =>
              Math.abs(second.paired.center) - Math.abs(first.paired.center),
          )
          .slice(0, 2)
          .map(
            (record) =>
              `${escape(record.name)}: ${interval(record.paired)}; ${record.initial}`,
          )
          .join("<br>") ||
        latest
          .filter((record) => /\/warm-frame$|\/export\d+$/.test(record.name))
          .slice(0, 2)
          .map(
            (record) =>
              `${escape(record.name)}: P50 ${(record.p50 / 1000).toFixed(3)} ms; P95 ${record.tailEnough ? (record.p95 / 1000).toFixed(3) + " ms" : "insufficient"}`,
          )
          .join("<br>") ||
        "No paired comparison";
      lines.push(
        `| [${escape(series.title)}](performance-runs.json) / ${escape(series.steps.at(-1).label)} | ${latestRuns} | ${latest.length} | ${[...outcomes].map(([status, count]) => `${escape(status)}: ${count}`).join("; ")} | ${latest.filter((record) => record.tailEnough).length}/${latest.length} | ${representative} |`,
      );
    }
    if (details) lines.push("", "### Step Details", "");
    for (const step of details ? series.steps : []) {
      lines.push(
        "#### " + escape(step.label),
        "",
        "**Change:** " +
          escape(step.change || "Initial baseline; no engine optimization."),
        "",
        "**Verification:** " + escape(step.verification || "Not supplied."),
        "",
        "**Interpretation / Regressions:** " +
          escape(step.notes || "No causal performance conclusion recorded."),
        "",
        "**Raw Runs:** " +
          (
            step.runs ||
            step.pairs.flatMap((pair) => [pair.control, pair.candidate])
          )
            .map((relative) => "`" + escape(relative) + "`")
            .join(", "),
        "",
      );
    }
  }
  lines.push(
    "",
    "## Raw Data And Detailed Tables",
    "",
    "The active manifest preserves registered runs, change, verification and regression notes. Run directories contain validated samples, configuration, executable provenance and reference images; they are local artifacts, not bundled benchmark promises. Removed tracked history remains recoverable from Git, not active measurement rows.",
    "",
    "Generate full per-case tables, MAD, incremental and initial deltas, P95 eligibility and step notes without expanding this page:",
    "",
    "```sh",
    "node benches/report.cjs --details --output target/performance/details.md",
    "```",
    "",
    "## Update Procedure",
    "",
    "1. Keep the initial run directories and frozen executable. Run the same cohort before and after each focused optimization.",
    "2. Register raw directories and change/verification/negative-result notes in [performance-runs.json](performance-runs.json). Use a new series for different workloads or hosts.",
    "3. Run `node benches/report.cjs`. This validates every registered run and rebuilds the concise conclusions table; use --details with a separate --output for full tables.",
    "",
  );
  return lines.join("\n");
}

function updateBenchmarks(source, report) {
  assert(
    source.split(REPORT_START).length === 2 &&
      source.split(REPORT_END).length === 2,
    "Benchmark document must contain exactly one report marker pair",
  );
  const start = source.indexOf(REPORT_START) + REPORT_START.length;
  const end = source.indexOf(REPORT_END);
  assert(start <= end, "Benchmark report markers are reversed");
  assert(report.startsWith("# Performance\n\n"), "Unexpected report heading");
  const embedded = report
    .slice("# Performance\n\n".length)
    .replace(/^(#{2,4}) /gm, "#$1 ")
    .trimEnd();
  return (
    source.slice(0, start) + "\n\n" + embedded + "\n\n" + source.slice(end)
  );
}

function selfTest() {
  assert.equal(improvement(100, 80), 20);
  assert.equal(improvement(80, 60), 25);
  assert.equal(improvement(100, 60), 40);
  assert.equal(improvement(100, 120), -20);
  assert.equal(improvement(0, 5), null);
  assert.equal(median([9, 1, 5]), 5);
  assert.equal(median([8, 2]), 5);
  assert.equal(
    stable({ second: 2, first: 1 }),
    stable({ first: 1, second: 2 }),
  );
  assert.equal(fingerprint(Buffer.from("hello")), "a430d84680aabd0b");
  const report = {
    schema: "motion-bench-v3",
    settings: { samples: 5 },
    references: { "frame.ppm": "reference" },
    execution: { started_unix_ns: "0", finished_unix_ns: "1" },
    records: [
      {
        name: "cpu/frame",
        metrics: {},
        measurement: {
          timing_kind: "single-operation",
          checksum: "0000000000000007",
          p50_us: 3,
          p95_us: 5,
          min_us: 1,
          max_us: 5,
          samples: [1, 2, 3, 4, 5].map((value) => ({
            elapsed_ns: value * 1000,
            iterations: 1,
            checksum: "0000000000000007",
            frame: null,
          })),
        },
      },
    ],
  };
  report.records[0].measurement.checksum = foldChecksum(
    report.records[0].measurement.samples.map((sample) => sample.checksum),
  );
  validate(report);
  const incorrect = structuredClone(report);
  incorrect.records[0].measurement.p50_us = 4;
  assert.throws(() => validate(incorrect), /raw samples/);
  assert.throws(() => validate({ ...report, records: [] }), /Empty report/);
  assert.equal(estimate([20, 20]).status, "exploratory (<5 pairs)");
  assert.equal(estimate([-2, -1, 0, 1, 2]).status, "uncertain");
  assert.equal(estimate([9, 10, 11, 12, 10]).status, "improved");
  assert.equal(estimate([-9, -10, -11, -12, -10]).status, "regressed");
  assert.equal(pairedEstimate([0], [1]).status, "unmeasurable (zero control)");
  assert.throws(
    () =>
      validate({
        ...report,
        records: [
          {
            ...report.records[0],
            measurement: {
              ...report.records[0].measurement,
              timing_kind: "frame-sequence",
            },
          },
        ],
      }),
    /Missing frame schedule/,
  );
  const sequence = structuredClone(report);
  sequence.configuration = { sequence_frames: 5 };
  sequence.records[0].measurement.timing_kind = "frame-sequence";
  sequence.records[0].measurement.samples.forEach(
    (sample, index) => (sample.frame = index),
  );
  validate(sequence);
  sequence.records[0].measurement.samples[1].frame = 0;
  assert.throws(() => validate(sequence), /permutation/);
  const temporary = fs.mkdtempSync(
    path.join(require("node:os").tmpdir(), "motion-statistics-"),
  );
  try {
    fs.writeFileSync(
      path.join(temporary, "frame.ppm"),
      Buffer.from("identical pixels"),
    );
    const runs = new Map();
    const entry = (name, time, start, executable) => {
      const value = structuredClone(report);
      value.settings = { samples: 5 };
      value.configuration = { size: [1, 1] };
      value.environment = { machine: "test" };
      value.suite = "test";
      value.provenance = { executable_fingerprint: executable };
      value.execution = {
        started_unix_ns: String(start),
        finished_unix_ns: String(start + 1),
      };
      const measured = value.records[0].measurement;
      for (const field of ["p50_us", "p95_us", "min_us", "max_us"])
        measured[field] = time;
      measured.samples.forEach((sample) => (sample.elapsed_ns = time * 1000));
      runs.set(name, { report: value, directory: temporary });
    };
    entry("initial", 100, 0, "A");
    entry("control-1", 100, 2, "A");
    entry("candidate-1", 80, 4, "B");
    entry("candidate-2", 80, 6, "B");
    entry("control-2", 100, 8, "A");
    entry("control-3", 80, 10, "B");
    entry("candidate-3", 60, 12, "C");
    entry("candidate-4", 60, 14, "C");
    entry("control-4", 80, 16, "B");
    const step = (label, offset) => ({
      label,
      change: "test",
      verification: "test",
      notes: "test",
      pairs: [
        {
          control: `control-${offset + 1}`,
          candidate: `candidate-${offset + 1}`,
          order: "control-first",
        },
        {
          control: `control-${offset + 2}`,
          candidate: `candidate-${offset + 2}`,
          order: "candidate-first",
        },
      ],
    });
    const history = {
      schema: MANIFEST_SCHEMA,
      series: [
        {
          id: "test",
          title: "test",
          steps: [
            { label: "00", runs: ["initial"] },
            step("01", 0),
            step("02", 2),
          ],
        },
      ],
    };
    const rendered = document(history, (name) => runs.get(name), true);
    assert(
      rendered.includes("+25.00% (exploratory) | +40.00%"),
      "Previous and initial improvements must stay distinct",
    );
    const concise = document(history, (name) => runs.get(name));
    const active = document(
      {
        schema: MANIFEST_SCHEMA,
        reset: { previous_series: 70 },
        baseline: {
          source: { commit: "test-commit", working_diff_sha256: "test-diff" },
          failures: [],
        },
        series: [
          {
            id: "baseline",
            title: "baseline",
            steps: [{ label: "00", runs: ["initial"] }],
          },
        ],
      },
      (name) => {
        const value = structuredClone(runs.get(name));
        value.report.records[0].name = "Cpu/empty/warm-frame";
        return value;
      },
    );
    assert(active.includes("P50 0.100 ms; P95 insufficient"));
    assert(
      active.includes("Source commit: test-commit") &&
        active.includes("Historical series cleared: 70"),
    );
    assert(concise.includes("exploratory (<5 pairs): 1"));
    assert(concise.includes("+25.00% (exploratory); +40.00%"));
    assert(!concise.includes("### Step Details"));
    assert.equal(
      concise.split("\n").filter((line) => line.startsWith("| ")).length,
      3,
    );
    const rankedRuns = new Map(
      [...runs].map(([name, value]) => [name, structuredClone(value)]),
    );
    for (const run of rankedRuns.values()) {
      const record = structuredClone(run.report.records[0]);
      record.name = "cpu/regression";
      if (run.report.provenance.executable_fingerprint === "C") {
        for (const field of ["p50_us", "p95_us", "min_us", "max_us"])
          record.measurement[field] = 200;
        record.measurement.samples.forEach(
          (sample) => (sample.elapsed_ns = 200000),
        );
      }
      run.report.records.push(record);
    }
    const ranked = document(history, (name) => rankedRuns.get(name));
    const summary = ranked
      .split("\n")
      .find((line) => line.startsWith("| [test]"));
    assert(
      summary.indexOf("cpu/regression: -150.00%") <
        summary.indexOf("cpu/frame: +25.00%"),
      "Representative changes must rank actual absolute medians and retain regressions",
    );
    const wrong = structuredClone(history);
    wrong.series[0].steps[1].pairs[1].order = "control-first";
    assert.throws(() => document(wrong, (name) => runs.get(name)), /alternate/);
    const overlap = structuredClone(runs.get("candidate-1"));
    overlap.report.execution.started_unix_ns = "2";
    runs.set("candidate-1", overlap);
    assert.throws(() => document(history, (name) => runs.get(name)), /overlap/);
  } finally {
    fs.rmSync(temporary, { recursive: true });
  }
  assert(
    document({ schema: MANIFEST_SCHEMA, series: [] }).includes("No baseline"),
  );
  const current = document({
    schema: MANIFEST_SCHEMA,
    series: [],
    measurement: {
      source_commit: "current-test-source",
      targets: ["performance", "breadth", "criterion_frames"],
      excluded_targets: ["films"],
      sampling: "One current invocation per target.",
      scope: "No historical improvement claim.",
      criterion: {
        directory: "target/criterion",
        records: [
          {
            name: "test",
            median_ns: 2000,
            lower_ns: 1000,
            upper_ns: 3000,
            samples: 10,
          },
        ],
      },
    },
  });
  assert(current.includes("Excluded: films"));
  assert(current.includes("current-test-source"));
  assert(current.includes("| test | 2.000 | 1.000 .. 3.000 | 10 |"));
  assert(!current.includes("Three independent runs"));
  const prefix = "# Benchmarks\n\nHandwritten guide.\n\n" + REPORT_START;
  const suffix = REPORT_END + "\n\nHandwritten footer.\n";
  const merged = updateBenchmarks(prefix + "\nOld report.\n" + suffix, current);
  assert(merged.startsWith(prefix + "\n\n"));
  assert(merged.endsWith("\n\n" + suffix));
  assert(merged.includes("### Interpretation\n"));
  assert(merged.includes("#### Criterion CPU Warm Frames\n"));
  assert(!merged.includes("# Performance\n"));
  assert(!merged.includes("Old report."));
  assert.equal(updateBenchmarks(merged, current), merged);
  assert.throws(() => updateBenchmarks("no markers", current), /exactly one/);
  assert.throws(
    () => updateBenchmarks(merged + REPORT_START, current),
    /exactly one/,
  );
  assert.throws(
    () => updateBenchmarks(REPORT_END + REPORT_START, current),
    /reversed/,
  );
  console.log(
    "PASS: statistics, fingerprints, report markers, guide preservation and idempotent updates",
  );
}

function main() {
  if (process.argv.includes("--self-test")) return selfTest();
  const args = process.argv.slice(2);
  let manifest = path.join(ROOT, "docs/performance-runs.json");
  let output = BENCHMARKS_DOCUMENT;
  let details = false;
  for (let index = 0; index < args.length; ) {
    if (args[index] === "--details") {
      details = true;
      index += 1;
      continue;
    }
    assert(args[index + 1], "Missing argument value");
    if (args[index] === "--manifest") manifest = path.resolve(args[index + 1]);
    else if (args[index] === "--output") output = path.resolve(args[index + 1]);
    else throw new Error("Unknown argument: " + args[index]);
    index += 2;
  }
  assert(
    !details || output !== BENCHMARKS_DOCUMENT,
    "Detailed tables require a separate --output",
  );
  const result = document(
    JSON.parse(fs.readFileSync(manifest, "utf8")),
    loadReport,
    details,
  );
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(
    output,
    output === BENCHMARKS_DOCUMENT
      ? updateBenchmarks(fs.readFileSync(output, "utf8"), result)
      : result,
  );
  console.log("Updated " + path.relative(ROOT, output));
}

module.exports = {
  document,
  updateBenchmarks,
  validate,
  verifyComparable,
  loadReport,
  estimate,
};
if (require.main === module)
  try {
    main();
  } catch (error) {
    console.error("Performance report rejected: " + error.message);
    process.exitCode = 1;
  }
