const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");

const policies = [
  [
    "motion-core/src/optics.rs",
    ["anamorphic sigmas"],
    "invariant",
    "streak_sigmas returns None only for non-anamorphic variants; the call is inside an Anamorphic match arm.",
  ],
  [
    "motion-render/src/cpu/filter.rs",
    ["anamorphic sigmas", "streak source"],
    "invariant",
    "The arm matches Optics::Anamorphic, and the streak source is captured before the anamorphic pass starts.",
  ],
  [
    "motion-render/src/gpu/plan/steps.rs",
    ["anamorphic sigmas"],
    "invariant",
    "streak_sigmas returns None only for non-anamorphic variants; the call is inside an Anamorphic match arm.",
  ],
  [
    "gallery/src/demos/particle_garden.rs",
    ["initialised above"],
    "invariant",
    "The OnceLock is filled immediately before the read when it was empty.",
  ],
  [
    "motion-render/src/look.rs",
    ["valid fixed halation"],
    "invariant",
    "The built-in recipe passes fixed finite nonnegative strengths/radii; halation only rejects invalid scalar parameters.",
  ],
  [
    "motion/src/project.rs",
    ["validated defaults"],
    "invariant",
    "Project parameter admission checks that defaults are objects before merging.",
  ],
  [
    "motion-content/src/image/animated.rs",
    ["nonempty frames", "last frame"],
    "invariant",
    "from_frames rejects empty input and constructs one cumulative end per retained frame; fields are private.",
  ],
  [
    "motion-content/src/image/static_image.rs",
    ["bounded image texel"],
    "invariant",
    "Coordinates are clamped to a nonempty validated image before sampling.",
  ],
  [
    "motion-content/src/particles/batch.rs",
    ["sprite material"],
    "invariant",
    "Only the sprite-material branch accesses the sprite configured during batch construction.",
  ],
  [
    "motion-content/src/particles/batch.rs",
    ["source texture bounds"],
    "invariant",
    "Sprite dimensions and texel count are validated; sampling bounds coordinates before lookup.",
  ],
  [
    "motion-content/src/vector/effects.rs",
    ["painted path"],
    "invariant",
    "The preceding path collection excludes paths without either a fill or stroke.",
  ],
  [
    "motion-content/src/vector.rs",
    ["one isolated content sample"],
    "invariant",
    "A single unit-weight sample is finite and satisfies Node::accumulate's admission contract.",
  ],
  [
    "motion-core/src/animation/keyframes.rs",
    ["keyframes contain an initial value"],
    "invariant",
    "Constructors install the initial keyframe and there is no public removal operation.",
  ],
  [
    "motion-core/src/animation.rs",
    ["animation duration overflow"],
    "convenience-panic",
    "The convenience animation combinator deliberately panics when duration addition overflows.",
  ],
  [
    "motion-core/src/asset.rs",
    ["typed resource key", "typed global key"],
    "invariant",
    "Resource keys include TypeId; only the matching Entry<A> is inserted under that key.",
  ],
  [
    "motion-core/src/authoring/context/cache.rs",
    ["located cache entry"],
    "invariant",
    "The index was found in this same deque while the cache lock remains held.",
  ],
  [
    "motion-core/src/authoring/layout.rs",
    ["validated baseline"],
    "invariant",
    "The earlier baseline-admission pass checks every nonempty child before placement uses the same indices.",
  ],
  [
    "motion-core/src/authoring/transition.rs",
    ["bounded built-in transition weights", "fixed zoom samples"],
    "invariant",
    "Built-in transitions supply a bounded fixed sample set and validate their input before constructing weights.",
  ],
  [
    "motion-core/src/color.rs",
    ["Invalid floating color"],
    "convenience-panic",
    "Color's floating convenience constructor intentionally panics; checked_channels returns a ValidationError.",
  ],
  [
    "motion-core/src/drawing/stroke.rs",
    ["checked subpath"],
    "invariant",
    "Path verb validation requires a MoveTo before Close; the local walk records that same subpath start.",
  ],
  [
    "motion-core/src/path2d/metrics.rs",
    [
      "Path arc-length calculation failed",
      "Path arc-length sampling failed",
      "Path trimming failed",
    ],
    "convenience-panic",
    "Public convenience geometry methods deliberately panic; their try_* variants return validation errors.",
  ],
  [
    "motion-core/src/path2d/metrics.rs",
    ["positive total length"],
    "invariant",
    "Zero-length paths return earlier; a positive total length requires at least one retained segment.",
  ],
  [
    "motion-core/src/path2d/morph.rs",
    ["active morph contour"],
    "invariant",
    "Validated drawing verbs occur only after MoveTo, which initializes the active contour.",
  ],
  [
    "motion-core/src/path2d/morph.rs",
    ["MorphPath interpolation failed"],
    "convenience-panic",
    "The convenience interpolation method intentionally panics; try_interpolate is the recoverable boundary.",
  ],
  [
    "motion-core/src/random.rs",
    ["slice length fits u64"],
    "invariant",
    "Supported targets have at most 64-bit usize, so a slice length fits u64.",
  ],
  [
    "motion-core/src/time.rs",
    [
      "frame time exceeds the signed tick range",
      "frame time exceeds the tick range",
      "frame index exceeds the signed 64-bit range",
      "frame index exceeds the unsigned 64-bit range",
      "duration exceeds the signed tick range",
      "time addition overflow",
      "time subtraction overflow",
      "duration addition overflow",
      "duration subtraction underflow",
      "duration multiplication overflow",
      "seconds overflow",
      "milliseconds overflow",
    ],
    "convenience-panic",
    "Time and Duration constructors, literals and arithmetic intentionally panic on tick overflow (or on subtracting past zero); checked arithmetic/conversion is used at I/O boundaries.",
  ],
  [
    "motion-core/src/time/offset.rs",
    [
      "offset addition overflow",
      "offset subtraction overflow",
      "offset negation overflow",
    ],
    "convenience-panic",
    "TimeOffset arithmetic intentionally panics on signed-tick overflow, like Time arithmetic.",
  ],
  [
    "motion-encode/src/audio/analysis.rs",
    ["finite PCM"],
    "invariant",
    "The analysis owns a finite prepared PCM buffer; its duration is defined by its validated sample count and rate.",
  ],
  [
    "motion-encode/src/audio/file_source.rs",
    ["Use AudioSource::try_read for recoverable file decoding"],
    "convenience-panic",
    "AudioSource::read is the convenience boundary; try_read propagates file/decoder errors.",
  ],
  [
    "motion-encode/src/audio/file_source.rs",
    ["file source"],
    "invariant",
    "Only the explicit Source::File branch dispatches to this private file-preparation helper.",
  ],
  [
    "motion-encode/src/encoder/gif.rs",
    ["opened GIF"],
    "invariant",
    "Successful open installs rate, encoder, temporary and started together with private options; write/finish reject absent options first.",
  ],
  [
    "motion-encode/src/encoder/png.rs",
    ["opened sequence"],
    "invariant",
    "Successful open installs directory, options and started; write/finish first require private options and do not reenter consumed finish.",
  ],
  [
    "motion-encode/src/encoder/gif.rs",
    ["normalized range"],
    "invariant",
    "open rejects a missing range before retaining an immutable options snapshot.",
  ],
  [
    "motion-encode/src/encoder/png.rs",
    ["normalized range"],
    "invariant",
    "open rejects a missing range before retaining an immutable options snapshot.",
  ],
  [
    "motion-encode/src/export.rs",
    ["normalized size", "normalized range"],
    "invariant",
    "Export normalization resolves missing size/range and validates them before entering the execution stages.",
  ],
  [
    "motion-encode/src/export/gpu.rs",
    ["normalized size"],
    "invariant",
    "The private GPU export stage is called only after export options normalization.",
  ],
  [
    "motion-encode/src/export/parallel.rs",
    ["initialized renderer"],
    "invariant",
    "The same closure installs a renderer when absent; failed construction returns before the access.",
  ],
  [
    "motion-render/src/composition/execution.rs",
    ["selected native GPU"],
    "invariant",
    "Backend selection installs the GPU before recording a successful GPU decision; failed Automatic selection resets it before recording a CPU decision.",
  ],
  [
    "motion-render/src/composition/execution/backend.rs",
    ["selected native GPU", "GPU initialized"],
    "invariant",
    "Backend selection installs the GPU before recording a successful GPU decision; failed Automatic selection resets it before recording a CPU decision.",
  ],
  [
    "motion-render/src/composition/execution.rs",
    ["enabled timings"],
    "invariant",
    "The local start instant is constructed only from Some(timings); the intervening stage does not disable timings.",
  ],
  [
    "motion-render/src/composition/execution/backend.rs",
    ["enabled timings"],
    "invariant",
    "The local start instant is constructed only from Some(timings); the intervening stage does not disable timings.",
  ],
  [
    "motion-render/src/composition/execution/preparation.rs",
    ["enabled timings"],
    "invariant",
    "The local start instant is constructed only from Some(timings); the intervening stage does not disable timings.",
  ],
  [
    "motion-render/src/composition/execution.rs",
    ["successful linear stage"],
    "invariant",
    "The preceding successful render stage records last_stats; its failure returns before consuming those stats.",
  ],
  [
    "motion-render/src/context.rs",
    ["new resource snapshot"],
    "invariant",
    "Shared assets are first replaced by a new unshared snapshot Arc before mutable access.",
  ],
  [
    "motion-render/src/cpu/filter.rs",
    ["shadow source", "glow source", "filter source"],
    "invariant",
    "The operation preparation phase retains original pixels for shadow/glow and source-reading custom filters before their matching execution branch.",
  ],
  [
    "motion-render/src/cpu/pool.rs",
    ["retained pixel buffer"],
    "invariant",
    "The loop condition establishes a retained buffer before popping the same private pool.",
  ],
  [
    "motion-render/src/cpu/walk.rs",
    [
      "validated root surface",
      "validated mask owner",
      "validated mask phase",
      "validated mask capture",
      "prepared layer balance",
      "prepared layer has a parent",
    ],
    "invariant",
    "Validated command balance drives the private surface stack; the initial root remains below all layer/mask captures.",
  ],
  [
    "motion-render/src/cpu/walk.rs",
    ["source filter scope"],
    "invariant",
    "The saved-source marker is created only for a filtered prepared layer; its immutable descriptor retains those filters.",
  ],
  [
    "motion-render/src/cpu.rs",
    ["validated parent", "validated root surface"],
    "invariant",
    "Prepared command balance preserves an initialized root surface and a parent for each child.",
  ],
  [
    "motion-render/src/gpu/context/textures.rs",
    ["shared shader arithmetic boundary", "shared arithmetic end"],
    "bundled-source",
    "Both markers belong to include_str! bundled shader sources; the device-free assembly/Naga test checks them.",
  ],
  [
    "motion-render/src/gpu/execute/particles.rs",
    ["prefix word"],
    "invariant",
    "Each mapped prefix component is sliced to exactly four bytes before u32 conversion.",
  ],
  [
    "motion-render/src/gpu/execute/pending.rs",
    ["four-byte readback component"],
    "invariant",
    "Readback components come from exact four-byte chunks after row-length and crop admission.",
  ],
  [
    "motion-render/src/gpu/execute/textures.rs",
    ["hardware blend capability"],
    "invariant",
    "The caller enables the hardware blend path only when TexturePipelines contains the blend pipelines.",
  ],
  [
    "motion-render/src/gpu/plan/items.rs",
    [
      "validated root layer",
      "validated root",
      "mask owner",
      "mask",
      "mask mode",
      "layer",
      "parent",
    ],
    "invariant",
    "Validated prepared commands maintain the private root/layer/mask stack; begin_mask initializes mask mode before end_mask.",
  ],
  [
    "motion-render/src/gpu/plan/items.rs",
    ["validated image transform"],
    "invariant",
    "Prepared image paint admission requires an invertible mapping; the planner reads that immutable mapping.",
  ],
  [
    "motion-render/src/gpu/plan/items.rs",
    ["fill operation"],
    "invariant",
    "The immediately preceding successful planner.fill appends a Fill step before assigning its payload/quads.",
  ],
  [
    "motion-render/src/gpu/plan/items.rs",
    ["source filter scope"],
    "invariant",
    "A saved-source index exists only on a filtered layer with an immutable filters descriptor.",
  ],
  [
    "motion-render/src/gpu/plan.rs",
    ["root"],
    "invariant",
    "Scheduling starts with one root layer; validated prepared commands leave that root on the stack.",
  ],
  [
    "motion-render/src/gpu/abi.rs",
    ["formatting a shader constant into a String cannot fail"],
    "invariant",
    "std::fmt::Write for String has no error-returning I/O operation; allocation failure is a separate process policy.",
  ],
  [
    "motion-render/src/raster/paint.rs",
    ["validated pattern"],
    "invariant",
    "Paint validation rejects singular pattern transforms before constructing the prepared Source.",
  ],
  [
    "motion-render/src/raster/paint.rs",
    ["prepared gradient"],
    "invariant",
    "Source construction creates gradient geometry whenever its immutable paint is a gradient.",
  ],
  [
    "motion-render/src/raster/polygon/shape_clip.rs",
    ["active clip interval"],
    "invariant",
    "The active-interval flag is set together with the preceding boundary in the same ordered sweep.",
  ],
  [
    "motion-render/src/raster/rect_clip.rs",
    ["nonempty convex polygon"],
    "invariant",
    "Empty polygons return before requesting the previous vertex from this same polygon.",
  ],
  [
    "motion-studio/src/audio/preview.rs",
    ["pending audio request"],
    "invariant",
    "The condvar loop holds the mailbox lock and exits only for a pending request or stopping; stopping returns first.",
  ],
  [
    "motion-studio/src/audio/preview.rs",
    ["prepared audio window"],
    "invariant",
    "The native-window branch is enabled only after installing its prepared window state.",
  ],
  [
    "motion-studio/src/preview/pool.rs",
    ["thumbnail cache", "preview cache", "preview texture cache"],
    "invariant",
    "An index is selected from the same coordinator-owned deque immediately before removal, without intervening mutation.",
  ],
  [
    "motion-studio/src/preview/worker.rs",
    ["oldest pending preview"],
    "invariant",
    "front_mut established a nonempty thread-owned queue before the same worker pops its front.",
  ],
  [
    "motion-studio/src/preview/worker.rs",
    ["pending preview"],
    "invariant",
    "The locked condvar loop exits only for pending work or stopping, and stopping is checked before take.",
  ],
  [
    "motion-studio/src/timeline_ui.rs",
    ["candidates"],
    "invariant",
    "Ruler-step candidates include a fixed nonempty fallback array.",
  ],
  [
    "motion-studio/src/viewer.rs",
    ["checker texture"],
    "invariant",
    "The preceding checker initialization installs the texture whenever the private cache is absent.",
  ],
  [
    "motion-content/src/image/static_image.rs",
    ["mip source texel in bounds"],
    "invariant",
    "The loops clamp every coordinate to the previous level's width and height.",
  ],
  [
    "motion-content/src/svg/convert.rs",
    ["filled path has a fill"],
    "invariant",
    "VectorPath::filled always sets a fill.",
  ],
  [
    "motion-content/src/vector/ink/brush/advanced/pigment.rs",
    ["Kubelka-Munk spectra"],
    "invariant",
    "The spectra are computed exactly when the mix mode is Kubelka-Munk, the only mode that reads them.",
  ],
  [
    "motion-render/src/cpu/blending.rs",
    ["nonseparable mix"],
    "invariant",
    "The nonseparable mix is computed for exactly the hue, saturation, color and luminosity modes matched here.",
  ],
  [
    "motion-render/src/gpu/execute/textures/uploads.rs",
    ["mapped staging buffer"],
    "invariant",
    "Every call is guarded by an installed staging buffer, which is created mapped together with its view.",
  ],
  [
    "motion-render/src/raster/paint.rs",
    ["gradient table has entries"],
    "invariant",
    "Gradient paints are validated to have at least two stops, and the table holds a row of entries per stop.",
  ],
  [
    "motion-render/src/raster/polygon.rs",
    ["inside span follows a crossing"],
    "invariant",
    "A span is inside only after a crossing has set the previous edge.",
  ],
];

function maskRust(source) {
  const output = source.split("");
  const blank = (start, end) => {
    for (let index = start; index < end; index++)
      if (output[index] !== "\n") output[index] = " ";
  };
  for (let index = 0; index < source.length;) {
    const start = index;
    if (source.startsWith("//", index)) {
      const end = source.indexOf("\n", index);
      index = end < 0 ? source.length : end;
    } else if (source.startsWith("/*", index)) {
      let depth = 1;
      index += 2;
      while (depth && index < source.length) {
        if (source.startsWith("/*", index)) {
          depth++;
          index += 2;
        } else if (source.startsWith("*/", index)) {
          depth--;
          index += 2;
        } else index++;
      }
      assert.equal(depth, 0, "Unterminated Rust block comment");
    } else {
      const raw =
        (index === 0 || !/\w/.test(source[index - 1])) &&
        source.slice(index).match(/^(?:b|c)?r(#{0,255})"/);
      if (raw) {
        const closing = '"' + raw[1];
        const end = source.indexOf(closing, index + raw[0].length);
        assert(end >= 0, "Unterminated raw Rust string");
        index = end + closing.length;
      } else if (source[index] === '"') {
        index++;
        while (index < source.length && source[index] !== '"')
          index += source[index] === "\\" ? 2 : 1;
        assert(index < source.length, "Unterminated Rust string");
        index++;
      } else {
        const character = source
          .slice(index)
          .match(/^'(?:\\(?:u\{[^}]+\}|x[\da-fA-F]{2}|.)|[^\\'])'/u);
        if (!character) {
          index++;
          continue;
        }
        index += character[0].length;
      }
    }
    blank(start, index);
  }
  return output.join("");
}

function closing(source, start, left, right) {
  let depth = 1;
  for (let index = start + 1; index < source.length; index++) {
    if (source[index] === left) depth++;
    if (source[index] === right && --depth === 0) return index + 1;
  }
  throw new Error("Unbalanced Rust delimiter");
}

function cfgValue(predicate) {
  predicate = predicate.trim();
  if (predicate === "test") return false;
  const call = predicate.match(/^(all|any|not)\s*\(([\s\S]*)\)$/);
  if (!call) return null;
  const argumentsList = [];
  let depth = 0,
    start = 0;
  for (let index = 0; index <= call[2].length; index++) {
    const value = call[2][index];
    if (value === "(") depth++;
    if (value === ")") depth--;
    if ((value === "," && depth === 0) || index === call[2].length) {
      const argument = call[2].slice(start, index).trim();
      if (argument) argumentsList.push(cfgValue(argument));
      start = index + 1;
    }
  }
  if (call[1] === "not")
    return argumentsList[0] === null ? null : !argumentsList[0];
  if (call[1] === "all")
    return argumentsList.includes(false)
      ? false
      : argumentsList.includes(null)
        ? null
        : true;
  return argumentsList.includes(true)
    ? true
    : argumentsList.includes(null)
      ? null
      : false;
}

function occurrences(source, pattern = /\.\s*expect\s*\(/g, literals = true) {
  const masked = maskRust(source),
    excluded = [];
  for (const match of masked.matchAll(/#\[\s*(cfg\s*\(|test\s*\])/g)) {
    const end = closing(masked, match.index + 1, "[", "]");
    const attribute = masked.slice(match.index + 2, end - 1).trim();
    if (attribute !== "test" && cfgValue(attribute.slice(4, -1)) !== false)
      continue;
    let item = end;
    while (/\s/.test(masked[item] || "")) item++;
    while (masked.startsWith("#[", item)) {
      item = closing(masked, item + 1, "[", "]");
      while (/\s/.test(masked[item] || "")) item++;
    }
    const brace = masked.indexOf("{", item),
      semi = masked.indexOf(";", item);
    if (brace >= 0 && (semi < 0 || brace < semi))
      excluded.push([item, closing(masked, brace, "{", "}")]);
  }
  return [...masked.matchAll(pattern)]
    .filter(
      (match) =>
        !excluded.some(
          ([start, end]) => start <= match.index && match.index < end,
        ),
    )
    .map((match) => {
      if (!literals)
        return {
          line: source.slice(0, match.index).split("\n").length,
          message: match[0],
          index: match.index,
        };
      const literal = source
        .slice(match.index)
        .match(/^\.\s*expect\s*\(\s*("(?:\\.|[^"\\])*")/);
      assert(literal, "Nonliteral expect requires explicit scanner support");
      return {
        line: source.slice(0, match.index).split("\n").length,
        message: JSON.parse(literal[1]),
      };
    });
}

function rustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory())
      return entry.name === "tests" ? [] : rustFiles(file);
    return file.endsWith(".rs") &&
      !/(?:^tests|_tests|snapshot)\.rs$/.test(entry.name)
      ? [file]
      : [];
  });
}

assert.deepEqual(
  occurrences(
    'fn live() { value.expect("live"); } #[cfg(test)] mod tests { value.expect("test"); }',
  ),
  [{ line: 1, message: "live" }],
);
assert.deepEqual(
  occurrences(
    '/* outer /* nested */ .expect("comment") */ fn live() { let text = r##".expect("raw")"##; value.expect("live"); }',
  ),
  [{ line: 1, message: "live" }],
);
assert.deepEqual(
  occurrences(
    '#[cfg(all(feature = "gpu", test))] fn helper() { value.expect("test"); }',
  ),
  [],
);
assert.deepEqual(
  occurrences('#[cfg(not(test))] fn live() { value.expect("live"); }'),
  [{ line: 1, message: "live" }],
);
assert.equal(cfgValue('any(test, feature = "gpu")'), null);

function audit() {
  const files = fs
    .readdirSync(path.join(root, "crates"))
    .flatMap((crate) => {
      const directory = path.join(root, "crates", crate, "src");
      return fs.existsSync(directory) ? rustFiles(directory) : [];
    })
    .concat(rustFiles(path.join(root, "gallery", "src")));
  const used = new Set();
  const entries = files.flatMap((file) => {
    const source = fs.readFileSync(file, "utf8");
    const relative = path.relative(root, file).split(path.sep).join("/");
    for (const [pattern, advice] of [
      [/\.\s*unwrap\s*\(\s*\)/g, "use expect with a registered invariant or return an error"],
      [/\bunreachable!\s*\(/g, "give unreachable! the invariant that makes it unreachable"],
    ]) {
      const found = occurrences(source, pattern, false).find(
        (entry) => !/^unreachable!\s*\(\s*"/.test(source.slice(entry.index)) ||
          !entry.message.startsWith("unreachable"),
      );
      assert(!found, `Production ${found?.message} at ${relative}:${found?.line}: ${advice}`);
    }
    return occurrences(source).map((entry) => {
      const relative = path.relative(root, file).split(path.sep).join("/");
      const policy = policies.find(
        ([suffix, messages]) =>
          relative.endsWith(suffix) && messages.includes(entry.message),
      );
      assert(
        policy,
        `Unassessed expect: ${relative}:${entry.line} ${entry.message}`,
      );
      used.add(`${policy[0]}\0${entry.message}`);
      return {
        file: relative,
        ...entry,
        decision: policy[2],
        reason: policy[3],
      };
    });
  });
  const stale = policies.flatMap(([suffix, messages]) =>
    messages
      .filter((message) => !used.has(`${suffix}\0${message}`))
      .map((message) => `${suffix} ${message}`),
  );
  assert.deepEqual(stale, [], "Stale expect policies");
  return entries;
}

if (require.main === module) {
  const entries = audit();
  const counts = entries.reduce((result, entry) => {
    result[entry.decision] = (result[entry.decision] || 0) + 1;
    return result;
  }, {});
  console.log(
    JSON.stringify(
      process.argv.includes("--json")
        ? {
          scope:
            "source-level, unexpanded macros, union of feature/target branches, test modules/files excluded",
          counts,
          entries,
        }
        : { assessed: entries.length, counts },
    ),
  );
}

module.exports = { maskRust, occurrences, audit };
