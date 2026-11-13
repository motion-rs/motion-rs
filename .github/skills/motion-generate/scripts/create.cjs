const fs = require("node:fs");
const path = require("node:path");
const { parseArgs } = require("node:util");

const root = path.resolve(__dirname, "../../../..");

function positiveInteger(value, name) {
  const number = Number(value);
  if (
    !/^\d+$/.test(String(value)) ||
    !Number.isSafeInteger(number) ||
    number <= 0 ||
    number > 0xffffffff
  ) {
    throw new Error(`${name} must be an integer between 1 and 4294967295`);
  }
  return number;
}

function createFile(filename, options = {}) {
  if (!filename || !filename.endsWith(".json"))
    throw new Error("Output must be a .json path");
  const output = path.resolve(filename);
  const size = String(options.size ?? "1920x1080").split("x");
  if (size.length !== 2) throw new Error("Size must be WIDTHxHEIGHT");
  const dimensions = size.map((value) => positiveInteger(value, "Size"));
  const fpsParts = String(options.fps ?? "30").split("/");
  if (fpsParts.length > 2)
    throw new Error("FPS must be an integer or NUMERATOR/DENOMINATOR");
  const numerator = positiveInteger(fpsParts[0], "FPS numerator");
  const denominator =
    fpsParts.length === 2 ? positiveInteger(fpsParts[1], "FPS denominator") : 1;
  const duration = Number(options.duration ?? 8);
  if (
    !Number.isFinite(duration) ||
    duration < denominator / numerator ||
    duration > 1e6
  ) {
    throw new Error(
      "Duration must be finite, at least one frame and at most 1000000 seconds",
    );
  }
  const name = options.name ?? path.basename(output, ".json");
  if (typeof name !== "string" || !name.trim())
    throw new Error("Name must not be empty");
  const schema = path.join(root, "schemas/motion.schema.json");
  if (!fs.statSync(schema).isFile())
    throw new Error("The committed schema is not a file");
  const directory = fs.realpathSync(path.dirname(output));
  const document = {
    $schema: path
      .relative(directory, fs.realpathSync(schema))
      .split(path.sep)
      .join("/"),
    version: 1,
    name,
    size: dimensions,
    fps: fpsParts.length === 2 ? `${numerator}/${denominator}` : numerator,
    background: "#14191f",
    duration,
  };
  if (options.seed !== undefined) {
    const seed = Number(options.seed);
    if (
      !/^\d+$/.test(String(options.seed)) ||
      !Number.isSafeInteger(seed) ||
      seed < 0
    ) {
      throw new Error("Seed must be an integer between 0 and 9007199254740991");
    }
    document.params = { seed };
  }
  document.tracks = [
    {
      name: "main",
      items: [{ scene: { name: "main", duration, content: { children: [] } } }],
    },
  ];
  fs.writeFileSync(output, `${JSON.stringify(document, null, 2)}\n`, {
    flag: "wx",
  });
  return output;
}

function main(args = process.argv.slice(2)) {
  const { values, positionals } = parseArgs({
    args,
    allowPositionals: true,
    options: {
      name: { type: "string" },
      size: { type: "string" },
      fps: { type: "string" },
      duration: { type: "string" },
      seed: { type: "string" },
      help: { type: "boolean" },
    },
  });
  if (values.help) {
    console.log(
      "Usage: node create.cjs FILE.json [--name TITLE] [--size 1920x1080] [--fps 30|30000/1001] [--duration 8] [--seed 17]",
    );
    console.log(
      "Creates a blank scene; parent directory must exist. Never overwrites. Seed declares params.seed for subsequent binding.",
    );
    return;
  }
  if (positionals.length !== 1)
    throw new Error("Provide exactly one output .json path; see --help");
  console.log(`Created ${createFile(positionals[0], values)}`);
}

module.exports = { createFile, main };

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`create: ${error.message}`);
    process.exitCode = 1;
  }
}
