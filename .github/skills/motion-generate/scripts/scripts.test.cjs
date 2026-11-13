const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");
const { createFile } = require("./create.cjs");
const { inspectFile, checkFile } = require("./check.cjs");

const root = path.resolve(__dirname, "../../../..");

function temporaryDirectory(context) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "motion-generate-"));
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

test("create writes a blank document with a correct schema path and defaults", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "new film.json");
  assert.equal(createFile(filename), filename);
  const bytes = fs.readFileSync(filename, "utf8");
  const document = JSON.parse(bytes);
  assert.equal(
    path.resolve(fs.realpathSync(directory), document.$schema),
    path.join(root, "schemas/motion.schema.json"),
  );
  assert.equal(document.version, 1);
  assert.equal(document.name, "new film");
  assert.deepEqual(document.size, [1920, 1080]);
  assert.equal(document.fps, 30);
  assert.equal(document.duration, 8);
  assert.deepEqual(document.tracks[0].items[0].scene.content, { children: [] });
  assert.equal(document.tracks[0].items[0].scene.duration, 8);
  assert.equal(document.params, undefined);
  assert.ok(bytes.endsWith("\n"));
});

test("create accepts explicit dimensions, rational FPS, duration and seed", (context) => {
  const filename = path.join(temporaryDirectory(context), "configured.json");
  createFile(filename, {
    name: "Configured",
    size: "1280x720",
    fps: "30000/1001",
    duration: "4.5",
    seed: "17",
  });
  const document = JSON.parse(fs.readFileSync(filename, "utf8"));
  assert.equal(document.name, "Configured");
  assert.deepEqual(document.size, [1280, 720]);
  assert.equal(document.fps, "30000/1001");
  assert.equal(document.duration, 4.5);
  assert.deepEqual(document.params, { seed: 17 });
  assert.equal(document.seed, undefined);
});

test("create rejects overwrite including symlinks without changing their targets", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "existing.json");
  fs.writeFileSync(filename, "keep this");
  assert.throws(() => createFile(filename), { code: "EEXIST" });
  const link = path.join(directory, "link.json");
  fs.symlinkSync(filename, link);
  assert.throws(() => createFile(link), { code: "EEXIST" });
  assert.equal(fs.readFileSync(filename, "utf8"), "keep this");
});

test("create rejects invalid inputs before writing", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "invalid.json");
  for (const options of [
    { size: "0x540" },
    { size: "960x540x1" },
    { size: "Infinityx540" },
    { fps: "0" },
    { fps: "30000/0" },
    { fps: "30/1/2" },
    { fps: "4294967296" },
    { duration: "NaN" },
    { duration: "0" },
    { duration: "0.0001" },
    { duration: "1000001" },
    { seed: "-1" },
    { seed: "9007199254740992" },
    { seed: "1.5" },
    { name: " " },
  ]) {
    assert.throws(() => createFile(filename, options));
    assert.equal(fs.existsSync(filename), false);
  }
  assert.throws(() => createFile(path.join(directory, "wrong.txt")));
  assert.throws(
    () => createFile(path.join(directory, "missing", "film.json")),
    { code: "ENOENT" },
  );
});

test("create CLI works outside the workspace and reports argument failures", (context) => {
  const directory = temporaryDirectory(context);
  const script = path.join(__dirname, "create.cjs");
  const invoke = (args) =>
    spawnSync(process.execPath, [script, ...args], {
      cwd: directory,
      encoding: "utf8",
    });
  assert.equal(invoke(["film.json", "--seed", "0"]).status, 0);
  assert.ok(fs.existsSync(path.join(directory, "film.json")));
  assert.equal(invoke(["--help"]).status, 0);
  assert.equal(invoke(["film.json"]).status, 1);
  assert.equal(invoke(["second.json", "--typo"]).status, 1);
  assert.equal(invoke(["one.json", "two.json"]).status, 1);
});

test("check preflights literal fonts, images, SVG, captions and audio without reading their content", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "film.json");
  createFile(filename);
  const document = JSON.parse(fs.readFileSync(filename, "utf8"));
  const sources = [
    "font.ttf",
    "image with spaces.png",
    "first.svg",
    "second.svg",
    "captions.srt",
    "audio.wav",
  ];
  sources.forEach((source) =>
    fs.writeFileSync(path.join(directory, source), "fixture"),
  );
  document.fonts = [sources[0]];
  document.audio = [{ source: sources[5] }];
  document.tracks[0].items[0].scene.content.children = [
    { image: { source: sources[1] } },
    { svg: { source: sources[2] } },
    { svg: { source: { data: "<svg/>" } } },
    { svg_morph: { from: sources[2], to: sources[3] } },
    { captions: { srt: sources[4] } },
    {
      particles: {
        emitters: [
          { image: { image: { size: [1, 1], rgba: [0, 0, 0, 255] } } },
        ],
      },
    },
  ];
  document.params = { unused: { image: { source: "unused.png" } } };
  fs.writeFileSync(filename, JSON.stringify(document));
  const result = inspectFile(filename);
  assert.equal(result.schema, path.join(root, "schemas/motion.schema.json"));
  assert.deepEqual(
    new Set(result.resources),
    new Set(
      sources.map((source) => fs.realpathSync(path.join(directory, source))),
    ),
  );
  assert.deepEqual(result.deferred, []);
});

test("check defers resource parameters and calculations rather than inventing resolution", (context) => {
  const filename = path.join(temporaryDirectory(context), "film.json");
  createFile(filename);
  const document = JSON.parse(fs.readFileSync(filename, "utf8"));
  document.fonts = [{ $param: "font" }];
  document.tracks[0].items[0].scene.content = {
    image: { source: { $calc: "$filename" } },
  };
  fs.writeFileSync(filename, JSON.stringify(document));
  assert.equal(inspectFile(filename).deferred.length, 2);
});

test("check rejects malformed JSON, invalid schema links and missing resource files", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "bad.json");
  for (const content of [
    "{",
    "[]",
    "null",
    '{"$schema":"missing.json"}',
    '{"$schema":"https://example.com/schema.json"}',
    '{"fonts":["missing.ttf"]}',
    '{"tracks":[{"image":{"source":"missing.png"}}]}',
  ]) {
    fs.writeFileSync(filename, content);
    assert.throws(() => inspectFile(filename));
  }
  fs.writeFileSync(path.join(directory, "not-schema.json"), "123");
  fs.writeFileSync(filename, '{"$schema":"not-schema.json"}');
  assert.throws(() => inspectFile(filename), /Schema file/);
  assert.throws(() => inspectFile(path.join(directory, "missing.json")), {
    code: "ENOENT",
  });
});

test("check bounds document bytes and nesting before invoking the engine", (context) => {
  const filename = path.join(temporaryDirectory(context), "bounded.json");
  fs.writeFileSync(filename, Buffer.alloc(16 * 1024 * 1024 + 1, 32));
  assert.throws(() => inspectFile(filename), /16 MiB/);
  let document = {};
  for (let depth = 0; depth < 130; depth++) document = { children: [document] };
  fs.writeFileSync(filename, JSON.stringify(document));
  assert.throws(() => inspectFile(filename), /128 nested/);
});

test("static-only checks never launch Cargo or create frame directories", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "film.json");
  createFile(filename);
  const forbiddenRun = () => assert.fail("static-only launched a process");
  const result = checkFile(filename, { staticOnly: true }, forbiddenRun);
  assert.equal(result.staticOnly, true);
  assert.deepEqual(result.frames, []);
  assert.throws(() =>
    checkFile(
      filename,
      {
        staticOnly: true,
        renderDir: path.join(directory, "frames"),
        times: "0",
      },
      forbiddenRun,
    ),
  );
  assert.equal(fs.existsSync(path.join(directory, "frames")), false);
});

test("check invokes CLI validation before CPU frames with shell-free absolute paths", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "film with spaces.json");
  const renderDir = path.join(directory, "frames with spaces");
  createFile(filename);
  const calls = [];
  const run = (command, args, options) => {
    calls.push({ command, args, options });
    return { status: 0 };
  };
  const result = checkFile(
    filename,
    {
      motion: path.join(directory, "motion executable"),
      renderDir,
      times: "0,2.5",
      size: "640x360",
    },
    run,
  );
  assert.equal(calls.length, 3);
  assert.deepEqual(calls[0].args, ["validate", filename]);
  assert.deepEqual(calls[1].args, [
    "still",
    filename,
    "--backend",
    "cpu",
    "--time",
    "0",
    "--size",
    "640x360",
    "--output",
    path.join(renderDir, "frame-01.png"),
  ]);
  assert.equal(calls[2].args[calls[2].args.indexOf("--time") + 1], "2.5");
  assert.equal(calls[0].options.cwd, root);
  assert.equal(calls[0].options.shell, false);
  assert.equal(calls[0].command, path.join(directory, "motion executable"));
  assert.equal(fs.statSync(renderDir).isDirectory(), true);
  assert.equal(result.frames.length, 2);
});

test("check defaults to the owning Cargo package and propagates process failures", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "film.json");
  const renderDir = path.join(directory, "frames");
  createFile(filename);
  let calls = 0;
  assert.throws(
    () =>
      checkFile(filename, { renderDir, times: "0" }, (command, args) => {
        calls++;
        assert.equal(command, "cargo");
        assert.deepEqual(args, [
          "run",
          "-p",
          "motion-cli",
          "--",
          "validate",
          filename,
        ]);
        return { status: 7 };
      }),
    { exitCode: 7 },
  );
  assert.equal(calls, 1);
  assert.equal(fs.existsSync(renderDir), false);
  assert.throws(
    () => checkFile(filename, {}, () => ({ status: null, signal: "SIGTERM" })),
    /SIGTERM/,
  );
  assert.throws(
    () =>
      checkFile(filename, {}, () => ({
        error: new Error("unavailable executable"),
      })),
    /unavailable/,
  );
});

test("check rejects unsafe render requests and existing directories before launching", (context) => {
  const directory = temporaryDirectory(context);
  const filename = path.join(directory, "film.json");
  createFile(filename);
  const forbiddenRun = () => assert.fail("invalid request launched a process");
  for (const options of [
    { renderDir: path.join(directory, "frames") },
    { times: "0" },
    { renderDir: directory, times: "0" },
    ...["-1", "NaN", "", "0,,1", "86401", Array(61).fill("0").join(",")].map(
      (times) => ({ renderDir: path.join(directory, "frames"), times }),
    ),
    { size: "0x540" },
    { size: "960x540x1" },
  ])
    assert.throws(() => checkFile(filename, options, forbiddenRun));
});

test("check CLI static mode runs outside the workspace and reports its limited acceptance", (context) => {
  const directory = temporaryDirectory(context);
  createFile(path.join(directory, "film.json"));
  const script = path.join(__dirname, "check.cjs");
  const invoke = (args) =>
    spawnSync(process.execPath, [script, ...args], {
      cwd: directory,
      encoding: "utf8",
    });
  const result = invoke(["film.json", "--static-only"]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(
    result.stdout,
    /Static-only: no schema constraints, engine preflight or pixels validated/,
  );
  assert.equal(invoke(["--help"]).status, 0);
  assert.equal(invoke(["film.json", "--typo"]).status, 1);
  assert.equal(invoke(["missing.json", "--static-only"]).status, 1);
});
