const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const root = path.resolve(__dirname, "..");
const matrix = [
  "",
  "vector",
  "image",
  "gpu",
  "particles",
  "gpu,particles",
  "gpu,particles,ffmpeg",
  "svg",
  "text",
  "typst",
  "formula",
  "ink",
  "diagram",
  "captions",
  "declarative",
  "declarative,svg",
  "declarative,particles",
  "declarative,diagram,captions",
  "declarative,formula",
  "declarative,audio",
  "audio",
  "ffmpeg",
  "studio",
  "default",
];

function manifest(features) {
  return `[package]
name = "motion-feature-consumer"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
motion = { package = "motion-rs", path = ${JSON.stringify(path.join(root, "crates/motion"))}, default-features = false, features = ${JSON.stringify(features)} }
`;
}

function options(argv) {
  let combinations = matrix;
  let selected = false;
  let offline = false;
  let selfTest = false;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--offline") offline = true;
    else if (argument === "--self-test") selfTest = true;
    else if (
      argument === "--features" &&
      !selected &&
      index + 1 < argv.length
    ) {
      const value = argv[++index];
      assert(
        value === "" || /^[a-z][a-z0-9-]*(,[a-z][a-z0-9-]*)*$/.test(value),
        "Expected comma-separated features",
      );
      combinations = [value];
      selected = true;
    } else throw new Error(`Unknown or incomplete argument: ${argument}`);
  }
  return { combinations, offline, selfTest };
}

function check(combination, offline) {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "motion-feature-consumer-"),
  );
  try {
    fs.mkdirSync(path.join(directory, "src"));
    const features = combination === "" ? [] : combination.split(",");
    fs.writeFileSync(path.join(directory, "Cargo.toml"), manifest(features));
    fs.writeFileSync(
      path.join(directory, "src/lib.rs"),
      `use motion::prelude::*;

    pub fn empty_node() -> Node { Node::empty() }

    pub fn unified_resources() -> motion::Result<bool> {
      struct Payload;
      impl motion::extend::Asset for Payload {
        const KIND: &'static str = "feature-payload";
        fn decode(_: &[u8], _: &motion::extend::LoadContext) -> motion::Result<Self> { Ok(Self) }
        fn retained_bytes(&self) -> usize { 8 }
      }
      let pool = motion::render::ResourceStore::with_limits(motion::render::ResourceLimits { max_entries: 8, max_bytes: 1 })?;
      let assets = Assets::new(".")?.with_store(pool.clone());
      let handle = assets.preload("payload", std::sync::Arc::new(Payload))?;
      let snapshot = assets.snapshot();
      assert!(assets.remove::<Payload>("payload"));
      assert!(std::sync::Arc::ptr_eq(&handle.get()?, &snapshot.load::<Payload>("payload").get()?));
      Ok(pool.retained_bytes() == 8)
    }

    pub fn author_scene() -> Result<Timeline, motion::ErrorReport> {
      Timeline::new(Size::new(8.0, 4.0), 30).track(|track| {
        track.scene(1_u32.seconds(), |context| -> motion::Result<_> {
          let path = Path2D::parse_svg("M0 0L2 0L2 2z")?;
          let cached = context.memo(key::F64(2.0), || path.fill(Color::WHITE));
          let progress = context.ease_in(Time::ZERO, 1_usize.seconds(), Easing::Linear);
          Ok((cached, Circle::new(context.center(), math::smoothstep(0.0, 1.0, progress))?.fill(Color::BLACK)))
        })
      }).build()
    }
    `,
    );
    const lock = path.join(root, "Cargo.lock");
    if (fs.existsSync(lock))
      fs.copyFileSync(lock, path.join(directory, "Cargo.lock"));
    const cargoArgs = [
      "check",
      "--manifest-path",
      path.join(directory, "Cargo.toml"),
      "-j",
      "2",
    ];
    if (offline) cargoArgs.push("--offline");
    console.log(`Checking isolated facade: ${combination || "minimal"}`);
    const result = spawnSync("cargo", cargoArgs, {
      cwd: directory,
      stdio: "inherit",
      env: {
        ...process.env,
        CARGO_TARGET_DIR:
          process.env.CARGO_TARGET_DIR ||
          path.join(root, "target/feature-consumer"),
        CARGO_INCREMENTAL: "0",
        CARGO_PROFILE_DEV_DEBUG: "0",
      },
    });
    if (result.error) throw result.error;
    assert.equal(
      result.status,
      0,
      `Facade check failed: ${combination || "minimal"} (${result.signal || result.status})`,
    );
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

const configuration = options(process.argv.slice(2));
if (configuration.selfTest) {
  assert.deepEqual(options(["--features", "", "--offline"]).combinations, [""]);
  assert.deepEqual(options(["--features", "gpu,particles"]).combinations, [
    "gpu,particles",
  ]);
  assert.throws(() => options(["--features"]));
  assert.throws(() => options(["--features", "--offline"]));
  assert.throws(() => options(["--features", "gpu", "--features", "text"]));
  assert.throws(() => options(["--unknown"]));
  assert(manifest(["gpu", "particles"]).includes("[workspace]"));
  assert(
    manifest(["gpu", "particles"]).includes(
      'default-features = false, features = ["gpu","particles"]',
    ),
  );
  assert.equal(new Set(matrix).size, matrix.length);
  console.log("Feature consumer self-test passed");
} else {
  for (const combination of configuration.combinations)
    check(combination, configuration.offline);
  console.log(
    `Passed ${configuration.combinations.length} isolated facade checks`,
  );
}
