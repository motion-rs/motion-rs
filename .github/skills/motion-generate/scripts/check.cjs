const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { parseArgs } = require("node:util");

const root = path.resolve(__dirname, "../../../..");
const maxJsonBytes = 16 * 1024 * 1024;

function readJson(filename) {
  const descriptor = fs.openSync(filename, "r");
  try {
    const stat = fs.fstatSync(descriptor);
    if (!stat.isFile() || stat.size > maxJsonBytes)
      throw new Error(`Expected a JSON file of at most 16 MiB: ${filename}`);
    const bytes = Buffer.alloc(stat.size + 1);
    let count = 0;
    while (count < bytes.length) {
      const received = fs.readSync(
        descriptor,
        bytes,
        count,
        bytes.length - count,
        null,
      );
      if (received === 0) break;
      count += received;
    }
    if (count > stat.size)
      throw new Error(`JSON file changed while reading: ${filename}`);
    return JSON.parse(bytes.subarray(0, count).toString("utf8"));
  } finally {
    fs.closeSync(descriptor);
  }
}

function inspectFile(filename) {
  if (!filename || !filename.endsWith(".json"))
    throw new Error("Input must be a .json path");
  const file = path.resolve(filename);
  const document = readJson(file);
  if (!document || typeof document !== "object" || Array.isArray(document))
    throw new Error("Document must be a JSON object");
  const directory = fs.realpathSync(path.dirname(file));
  const resources = new Set();
  const deferred = [];
  const localFile = (source, label) => {
    if (typeof source !== "string" || !source)
      throw new Error(`${label} must be a nonempty local file path`);
    if (/^[a-z][a-z0-9+.-]*:/i.test(source))
      throw new Error(
        `${label} must be local; this script does not fetch URLs`,
      );
    const resolved = path.resolve(directory, source);
    if (!fs.statSync(resolved).isFile())
      throw new Error(`${label} is not a file: ${resolved}`);
    fs.accessSync(resolved, fs.constants.R_OK);
    return fs.realpathSync(resolved);
  };
  const sourceFile = (source, label) => {
    if (typeof source === "string") resources.add(localFile(source, label));
    else if (source !== undefined) deferred.push(label);
  };
  let schema = null;
  if (document.$schema !== undefined) {
    schema = localFile(document.$schema, "$schema");
    const definition = readJson(schema);
    if (
      typeof definition !== "boolean" &&
      (!definition ||
        typeof definition !== "object" ||
        Array.isArray(definition))
    ) {
      throw new Error("Schema file must contain a JSON object or boolean");
    }
  }
  if (Array.isArray(document.fonts))
    document.fonts.forEach((source, index) =>
      sourceFile(source, `fonts[${index}]`),
    );
  else if (document.fonts !== undefined) deferred.push("fonts");
  if (Array.isArray(document.audio))
    document.audio.forEach((clip, index) =>
      sourceFile(clip?.source, `audio[${index}].source`),
    );
  else if (document.audio !== undefined) deferred.push("audio");
  const pending = [{ value: document, label: "document", depth: 0 }];
  let visited = 0;
  while (pending.length) {
    if (++visited > 100000)
      throw new Error("Static inspection is limited to 100000 JSON values");
    const { value, label, depth } = pending.pop();
    if (depth > 128)
      throw new Error("Static inspection is limited to 128 nested levels");
    if (!value || typeof value !== "object") continue;
    sourceFile(value.image?.source, `${label}.image.source`);
    if (
      typeof value.svg?.source === "string" ||
      value.svg?.source?.$param ||
      value.svg?.source?.$calc
    ) {
      sourceFile(value.svg.source, `${label}.svg.source`);
    }
    for (const endpoint of ["from", "to"]) {
      const source = value.svg_morph?.[endpoint];
      if (typeof source === "string" || source?.$param || source?.$calc)
        sourceFile(source, `${label}.svg_morph.${endpoint}`);
    }
    sourceFile(value.captions?.srt, `${label}.captions.srt`);
    for (const [key, child] of Object.entries(value)) {
      if (value === document && key === "params") continue;
      if (pending.length >= 100000)
        throw new Error(
          "Static inspection is limited to 100000 pending JSON values",
        );
      pending.push({
        value: child,
        label: `${label}.${key}`,
        depth: depth + 1,
      });
    }
  }
  return { file, schema, resources: [...resources], deferred };
}

function checkFile(filename, options = {}, run = spawnSync) {
  if (
    options.staticOnly &&
    (options.renderDir !== undefined || options.times !== undefined)
  ) {
    throw new Error("--static-only cannot render frames");
  }
  if ((options.renderDir !== undefined) !== (options.times !== undefined)) {
    throw new Error("Provide both --render-dir and --times to render frames");
  }
  const times =
    options.times === undefined
      ? []
      : String(options.times)
        .split(",")
        .map((value) => {
          const seconds = Number(value);
          if (
            !value.trim() ||
            !Number.isFinite(seconds) ||
            seconds < 0 ||
            seconds > 86400
          ) {
            throw new Error(
              "Frame times must be finite seconds between 0 and 86400",
            );
          }
          return seconds;
        });
  if (times.length > 60)
    throw new Error("Render at most 60 explicitly selected frames");
  const size = options.size ?? "960x540";
  if (
    !/^\d+x\d+$/.test(size) ||
    !size
      .split("x")
      .every((value) => Number(value) > 0 && Number(value) <= 0xffffffff)
  ) {
    throw new Error(
      "Size must be positive WIDTHxHEIGHT integers no larger than 4294967295",
    );
  }
  const renderDirectory =
    options.renderDir === undefined ? null : path.resolve(options.renderDir);
  if (renderDirectory && fs.existsSync(renderDirectory))
    throw new Error(
      "Render directory must not already exist; never overwrites",
    );
  const inspection = inspectFile(filename);
  if (options.staticOnly)
    return { ...inspection, staticOnly: true, frames: [] };
  const command = options.motion ? path.resolve(options.motion) : "cargo";
  const prefix = options.motion ? [] : ["run", "-p", "motion-cli", "--"];
  const invoke = (args) => {
    const result = run(command, [...prefix, ...args], {
      cwd: root,
      stdio: "inherit",
      shell: false,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      const error = new Error(
        `motion ${args[0]} failed (${result.signal ?? `exit ${result.status}`})`,
      );
      error.exitCode =
        Number.isInteger(result.status) &&
          result.status > 0 &&
          result.status <= 255
          ? result.status
          : 1;
      throw error;
    }
  };
  invoke(["validate", inspection.file]);
  const frames = [];
  if (renderDirectory) {
    fs.mkdirSync(renderDirectory);
    times.forEach((seconds, index) => {
      const output = path.join(
        renderDirectory,
        `frame-${String(index + 1).padStart(2, "0")}.png`,
      );
      invoke([
        "still",
        inspection.file,
        "--backend",
        "cpu",
        "--time",
        String(seconds),
        "--size",
        size,
        "--output",
        output,
      ]);
      frames.push(output);
    });
  }
  return { ...inspection, staticOnly: false, frames };
}

function main(args = process.argv.slice(2)) {
  const { values, positionals } = parseArgs({
    args,
    allowPositionals: true,
    options: {
      "static-only": { type: "boolean" },
      motion: { type: "string" },
      "render-dir": { type: "string" },
      times: { type: "string" },
      size: { type: "string" },
      help: { type: "boolean" },
    },
  });
  if (values.help) {
    console.log(
      "Usage: node check.cjs FILE.json [--static-only | --motion EXECUTABLE] [--render-dir NEW_DIR --times 0,2,4 --size 960x540]",
    );
    console.log(
      "Checks JSON, local schema link and literal resource files; not JSON Schema validation. Inputs/schema: max 16 MiB, nesting: 128 levels, values: 100000.",
    );
    console.log(
      "Default: cargo run -p motion-cli -- validate (may compile). --motion uses an existing executable. --static-only skips the engine and never renders.",
    );
    console.log(
      "Rendering requires a new directory with an existing parent, at most 60 times within the film, and never overwrites. Parameter/calculation references require engine validation.",
    );
    return;
  }
  if (positionals.length !== 1)
    throw new Error("Provide exactly one input .json path; see --help");
  const result = checkFile(positionals[0], {
    staticOnly: values["static-only"],
    motion: values.motion,
    renderDir: values["render-dir"],
    times: values.times,
    size: values.size,
  });
  console.log(
    `Local checks passed: JSON parsed; schema link ${result.schema ? "checked" : "not declared"}; ${result.resources.length} literal resource files checked.`,
  );
  if (result.deferred.length)
    console.log(
      `${result.deferred.length} resource references deferred to the engine.`,
    );
  console.log(
    result.staticOnly
      ? "Static-only: no schema constraints, engine preflight or pixels validated."
      : `Engine validation passed; ${result.frames.length} CPU frame(s) rendered. Inspect the images separately.`,
  );
  for (const frame of result.frames) console.log(frame);
}

module.exports = { inspectFile, checkFile, main };

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`check: ${error.message}`);
    process.exitCode = error.exitCode ?? 1;
  }
}
