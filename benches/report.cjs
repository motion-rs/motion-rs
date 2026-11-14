const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");

const ROOT = path.resolve(__dirname, "..");

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

function pairedRuns(step, load) {
  assert(
    Array.isArray(step.pairs) && step.pairs.length > 0,
    "A comparison requires fresh control/candidate pairs",
  );
  const seen = new Set(),
    candidates = [],
    controls = [];
  let first = null;
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
    first ??= control;
    verifyComparable(first, control);
    verifyComparable(first, candidate);
    assert.equal(
      control.report.provenance.executable_fingerprint,
      first.report.provenance.executable_fingerprint,
      "Control executable changed within a comparison",
    );
    if (candidates.length)
      assert.equal(
        candidate.report.provenance.executable_fingerprint,
        candidates[0].report.provenance.executable_fingerprint,
        "Candidate executable changed within a comparison",
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

function analyze(step, load = loadReport) {
  const { candidates, controls } = pairedRuns(step, load);
  const outcomes = new Map();
  const data = [];
  for (const original of controls[0].report.records) {
    const find = (run) =>
      run.report.records.find((record) => record.name === original.name);
    const control = controls.map(find),
      candidate = candidates.map(find);
    const quantile = (records, field) =>
      records.map((record) => record.measurement[field]);
    const paired = pairedEstimate(
      quantile(control, "p50_us"),
      quantile(candidate, "p50_us"),
    );
    const tailEnough = control.concat(candidate).every(
      (record) =>
        !["batch-mean", "mean-of-individual-operations"].includes(
          record.measurement.timing_kind,
        ) &&
        record.measurement.samples.length >= 100 &&
        record.measurement.samples.every((sample) => sample.iterations === 1),
    );
    const tail = tailEnough
      ? interval(
        pairedEstimate(
          quantile(control, "p95_us"),
          quantile(candidate, "p95_us"),
        ),
      )
      : "insufficient";
    outcomes.set(paired.status, (outcomes.get(paired.status) || 0) + 1);
    data.push({
      name: original.name,
      control: median(quantile(control, "p50_us")),
      candidate: median(quantile(candidate, "p50_us")),
      paired,
      tail,
      spreads: [
        spread(quantile(control, "p50_us")),
        spread(quantile(candidate, "p50_us")),
      ],
    });
  }
  return { data, outcomes, pairs: step.pairs.length };
}

function compare(step, load = loadReport) {
  const { data, outcomes } = analyze(step, load);
  const rows = data.map(
    (row) =>
      `| ${escape(row.name)} | ${row.control.toFixed(3)} | ${row.candidate.toFixed(3)} | ${interval(row.paired)} | ${row.paired.status} | ${row.tail} | ${row.spreads[0]} / ${row.spreads[1]} |`,
  );
  return [
    "# Performance Comparison",
    "",
    step.label ? `Label: ${escape(step.label)}.` : "",
    `Pairs: ${step.pairs.length}. Cases and phases: ${rows.length}. Outcomes: ${[...outcomes].map(([status, count]) => `${escape(status)}: ${count}`).join("; ")}.`,
    "",
    "- Change = `(control - candidate) / control * 100%` on paired P50 or P95. Positive is faster; negative is a regression.",
    "- Pairs alternate control-first and candidate-first. Paired changes have a deterministic bootstrap interval after at least five pairs; an interval crossing zero is uncertain.",
    "- P95 requires at least 100 individual samples in every run; otherwise it is insufficient.",
    "- Configuration, reference pixels, checksums, adapter and frozen executables are verified before comparing. No cross-case average is reported.",
    "",
    "| Case / Phase | Control P50 us | Candidate P50 us | P50 change (95% interval) | Assessment | P95 change | MAD% control / candidate |",
    "| --- | ---: | ---: | --- | --- | --- | --- |",
    ...rows,
    "",
  ].join("\n");
}

function selfTest() {
  assert.equal(improvement(100, 80), 20);
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
    entry("control-1", 100, 2, "A");
    entry("candidate-1", 80, 4, "B");
    entry("candidate-2", 80, 6, "B");
    entry("control-2", 100, 8, "A");
    const step = {
      label: "test",
      pairs: [
        { control: "control-1", candidate: "candidate-1", order: "control-first" },
        { control: "control-2", candidate: "candidate-2", order: "candidate-first" },
      ],
    };
    const rendered = compare(step, (name) => runs.get(name));
    assert(rendered.includes("| cpu/frame | 100.000 | 80.000 | +20.00% (exploratory) |"));
    assert(rendered.includes("exploratory (<5 pairs): 1"));
    const wrong = structuredClone(step);
    wrong.pairs[1].order = "control-first";
    assert.throws(() => compare(wrong, (name) => runs.get(name)), /alternate/);
    assert.throws(
      () => compare({ pairs: [] }, (name) => runs.get(name)),
      /requires fresh/,
    );
    const overlap = structuredClone(runs.get("candidate-1"));
    overlap.report.execution.started_unix_ns = "2";
    runs.set("candidate-1", overlap);
    assert.throws(() => compare(step, (name) => runs.get(name)), /overlap/);
  } finally {
    fs.rmSync(temporary, { recursive: true });
  }
  console.log("PASS: statistics, validation and paired comparison");
}

function main() {
  if (process.argv.includes("--self-test")) return selfTest();
  const args = process.argv.slice(2),
    options = new Map();
  for (let index = 0; index < args.length; index += 2) {
    assert(
      ["--pairs", "--output"].includes(args[index]) && args[index + 1],
      "Usage: node benches/report.cjs --pairs <pair-directory> [--output <file>]",
    );
    assert(!options.has(args[index]), "Repeated argument");
    options.set(args[index], args[index + 1]);
  }
  assert(options.has("--pairs"), "Missing --pairs");
  const step = JSON.parse(
    fs.readFileSync(
      path.resolve(ROOT, options.get("--pairs"), "step.json"),
      "utf8",
    ),
  );
  const result = compare(step);
  if (options.has("--output")) {
    const output = path.resolve(ROOT, options.get("--output"));
    fs.mkdirSync(path.dirname(output), { recursive: true });
    fs.writeFileSync(output, result);
    console.log("Wrote " + path.relative(ROOT, output));
  } else process.stdout.write(result);
}

module.exports = { analyze, compare, estimate, loadReport, validate, verifyComparable };
if (require.main === module)
  try {
    main();
  } catch (error) {
    console.error("Performance report rejected: " + error.message);
    process.exitCode = 1;
  }
