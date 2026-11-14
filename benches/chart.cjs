const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const { analyze } = require("./report.cjs");
const ROOT = path.resolve(__dirname, "..");
const COLORS = {
  improved: "#2e9e5b",
  regressed: "#d64545",
  neutral: "#8a94a6",
};

function xml(value) {
  return String(value).replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
}

// Horizontal bars of the paired P50 change per measurement, with the bootstrap
// interval as a whisker. Positive means the candidate is faster.
function chart(analysis, label, phase) {
  const rows = analysis.data
    .filter((row) => row.name.endsWith("/" + phase) && row.paired.center !== null)
    .sort((a, b) => b.paired.center - a.paired.center);
  assert(rows.length > 0, `No measurements for phase "${phase}"`);
  const count = (status) => rows.filter((row) => row.paired.status === status).length;
  const best = rows[0],
    worst = rows[rows.length - 1];
  const name = (row) => row.name.slice(0, -phase.length - 1);
  const summary = `Improved ${count("improved")}, regressed ${count("regressed")}, uncertain ${rows.length - count("improved") - count("regressed")} of ${rows.length}. Best: ${name(best)} ${best.paired.center.toFixed(1)}%. Worst: ${name(worst)} ${worst.paired.center.toFixed(1)}%.`;
  const left = 260,
    right = 220,
    plot = 520,
    top = 90,
    step = 22;
  const lows = rows.map((row) => row.paired.low ?? row.paired.center),
    highs = rows.map((row) => row.paired.high ?? row.paired.center);
  let min = Math.min(0, ...lows),
    max = Math.max(0, ...highs);
  const pad = (max - min || 1) * 0.08;
  min -= pad;
  max += pad;
  const x = (value) => left + ((value - min) / (max - min)) * plot;
  const width = left + plot + right,
    height = top + rows.length * step + 40;
  const out = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="Helvetica, Arial, sans-serif" font-size="12">`,
    `<rect width="100%" height="100%" fill="#ffffff"/>`,
    `<text x="20" y="28" font-size="16" font-weight="bold">${xml(label || "Performance comparison")}: ${xml(phase)} P50 change</text>`,
    `<text x="20" y="48" fill="#555">Paired runs: ${analysis.pairs}. Positive = candidate faster. Whiskers: 95% bootstrap interval (needs 5+ pairs).</text>`,
    `<text x="20" y="68" font-weight="bold">${xml(summary)}</text>`,
  ];
  const ticks = 6;
  for (let tick = 0; tick <= ticks; tick++) {
    const value = min + ((max - min) * tick) / ticks;
    out.push(
      `<line x1="${x(value)}" y1="${top - 6}" x2="${x(value)}" y2="${height - 30}" stroke="#eee"/>`,
      `<text x="${x(value)}" y="${height - 14}" text-anchor="middle" fill="#555">${value.toFixed(0)}%</text>`,
    );
  }
  out.push(
    `<line x1="${x(0)}" y1="${top - 6}" x2="${x(0)}" y2="${height - 30}" stroke="#333"/>`,
  );
  rows.forEach((row, index) => {
    const y = top + index * step,
      mid = y + step / 2,
      value = row.paired.center;
    const color = COLORS[row.paired.status] || COLORS.neutral;
    const name = row.name.slice(0, -phase.length - 1);
    out.push(
      `<text x="${left - 8}" y="${mid + 4}" text-anchor="end">${xml(name)}</text>`,
      `<rect x="${Math.min(x(0), x(value))}" y="${y + 3}" width="${Math.abs(x(value) - x(0))}" height="${step - 6}" fill="${color}"/>`,
    );
    if (row.paired.low !== null)
      out.push(
        `<line x1="${x(row.paired.low)}" y1="${mid}" x2="${x(row.paired.high)}" y2="${mid}" stroke="#222"/>`,
        `<line x1="${x(row.paired.low)}" y1="${mid - 4}" x2="${x(row.paired.low)}" y2="${mid + 4}" stroke="#222"/>`,
        `<line x1="${x(row.paired.high)}" y1="${mid - 4}" x2="${x(row.paired.high)}" y2="${mid + 4}" stroke="#222"/>`,
      );
    out.push(
      `<text x="${left + plot + 10}" y="${mid + 4}" fill="#333">${value >= 0 ? "+" : ""}${value.toFixed(1)}%  ${(row.control / 1000).toFixed(2)} → ${(row.candidate / 1000).toFixed(2)} ms</text>`,
    );
  });
  out.push("</svg>", "");
  return out.join("\n");
}

function main() {
  const args = process.argv.slice(2),
    options = new Map();
  for (let index = 0; index < args.length; index += 2) {
    assert(
      ["--pairs", "--output", "--phase"].includes(args[index]) && args[index + 1],
      "Usage: node benches/chart.cjs --pairs <pair-directory> --output <file.svg> [--phase warm-frame]",
    );
    options.set(args[index], args[index + 1]);
  }
  assert(options.has("--pairs") && options.has("--output"), "Missing --pairs or --output");
  const step = JSON.parse(
    fs.readFileSync(path.resolve(ROOT, options.get("--pairs"), "step.json"), "utf8"),
  );
  const svg = chart(analyze(step), step.label, options.get("--phase") || "warm-frame");
  const output = path.resolve(ROOT, options.get("--output"));
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, svg);
  console.log("Wrote " + path.relative(ROOT, output));
}

module.exports = { chart };
if (require.main === module)
  try {
    main();
  } catch (error) {
    console.error("Chart rejected: " + error.message);
    process.exitCode = 1;
  }
