"use strict";

const expr = (expr) => ({ expr });
const progress = "@clock.rotate";
const reveal = expr(`min(0.08 + ${progress} * 1.1, 1)`);
const seed = { $param: "ink_seed" };
const degrees = (radians) => radians * 180 / Math.PI;
const rect = (bounds, fill) => ({ rect: { bounds }, fill });
const text = (content, size, left, top, fill = "#17231f", family = "") => ({
  text: { content, family, size, weight: 500, letter_spacing: 0 },
  fill,
  translate: [left, top],
});
const linear = (start, end, stops) => ({
  linear: {
    start, end, options: { units: "user_space" },
    stops: stops.map(([offset, color]) => ({ offset, color })),
  },
});
const path = "M48 190 C130 18 390 285 280 150 C170 10 350 85 464 190";
const lifted = "M48 180 C120 30 160 240 230 140 M285 180 C330 40 400 235 465 130";
const pages = [
  ["Pressure curve", "Ordered pressure controls", "vary the nib width along", "the complete path length."],
  ["Pressure function", "A pure analytic function", "drives repeated width pulses", "without recorded state."],
  ["Fixed nib", "An elliptical nib keeps", "a fixed angle while the", "path changes direction."],
  ["Following nib", "The same elliptical nib", "rotates with the tangent", "plus an explicit offset."],
  ["Seeded bristles", "Five offset bristles share", "one continuous path with", "seeded spread and jitter."],
  ["Endpoint caps", "A tapered entry and flat", "exit alter the nib imprint", "at the path endpoints."],
  ["Gradient paint", "A spatial gradient replaces", "solid paint before material", "coverage is attenuated."],
  ["Ink depletion", "Load decreases along the", "full stroke, revealing dry", "gaps without path splitting."],
  ["Dry fibers", "Dry coverage follows path", "arc length and transverse", "distance across the nib."],
  ["Grain and edges", "Seeded granulation mottles", "coverage while roughness", "displaces the geometric rim."],
  ["Paper texture", "A generated height texture", "repeats under the stroke", "and attenuates pigment."],
  ["Intersection build-up", "Coverage from crossing", "segments adds density", "inside one retained stroke."],
  ["Rim darkening", "The stroke's geometric rim", "receives darker color with", "directional material detail."],
  ["Path wobble", "A fixed seeded displacement", "adds irregularity without", "changing on replay."],
  ["Frame-seeded boil", "Explicit frame numbers", "change wobble every two", "frames without history."],
  ["Analytic wetness", "Age expands the wash halo", "according to drying time;", "no fluid solver is involved."],
  ["Recorded input", "Position, pressure and tilt", "retain recorded timestamps", "including a stationary pause."],
  ["Timed pen lifts", "A monotone timing curve", "and explicit pen-up pause", "control separate subpaths."],
  ["Brush eraser", "The written stroke becomes", "an inverted-alpha mask", "over the colored source."],
  ["Pigment mixing", "Linear-light mixing and", "an approximate six-band", "pigment model are compared."],
  ["Height and relight", "A separate grayscale ridge", "drives surface lighting", "above the colored stroke."],
];

function brush(extra = {}) {
  return {
    brush_stroke: {
      path, width: 28, paint: "#167d71",
      material_work_limit: 128000000, write: reveal, ...extra,
    },
  };
}

function inkFeatures(document) {
  const treatments = [
    { pressure_curve: [[0, 0.15], [0.25, 1.8], [0.55, 0.3], [0.8, 1.5], [1, 0.1]] },
    { pressure_from: "0.3 + 0.9 * pow(sin(t * tau * 3), 2)" },
    { nib_aspect: 0.18, nib_mode: { fixed: degrees(0.7) } },
    { nib_aspect: 0.18, nib_mode: { follow_path: degrees(0.5) } },
    { sample_limit: 96, bristles: { count: 5, spread: 0.8, jitter: 0.12, seed }, ink: { seed, dryness: 0.25 } },
    { caps: [{ tapered: 75 }, "flat"] },
    { paint: linear([40, 0], [472, 0], [[0, "#3564c4"], [0.5, "#d64b36"], [1, "#e4aa37"]]) },
    { ink_load: [1, 0.05], ink: { seed, dryness: 0.3, fibers: 1 } },
    { ink: { seed, dryness: 0.75, fibers: 1 } },
    { ink: { seed, granulation: 0.8, edge_roughness: 3 } },
    { ink: { seed: 0, paper: { noise_texture: { size: [16, 16], seed, grain_scale: 2, absorbency: 0.8 } } } },
    { ink: { seed, granulation: 0.35, build_up: 0.8 } },
    { ink: { seed: 0, edge_darkening: 0.9, edge_anisotropy: 3 } },
    { wobble: { amplitude: 5, frequency: 0.06, seed } },
    { wobble: { amplitude: 3, frequency: 0.05, seed, frame: expr(`floor(${progress} * 120)`) } },
    { wetness: { age: expr(`${progress} * 4`), radius: 10, drying: 1.4 }, write: undefined },
    {
      path: undefined, width: 26, paint: "#3564c4", write: undefined,
      samples: [
        [48, 190, 0.2, 0], [140, 100, 1.4, 0.8], [256, 160, 0.6, 1.5],
        [256, 160, 0.6, 2.5], [380, 105, 1.3, 3.3], [464, 190, 0.1, 4],
      ].map(([x, y, pressure, time]) => ({ position: [x, y], pressure, tilt: degrees(time * 0.5), time })),
      nib_aspect: 0.35, nib_mode: "tilt", write_time: expr(`${progress} * 4`),
    },
    {
      path: lifted, write: undefined, write_time: expr(`${progress} * 3.45`),
      timing: { duration: 2.8, lift_pause: 0.65, curve: [[0, 0], [0.25, 0.1], [0.65, 0.85], [1, 1]] },
    },
    {}, {}, {},
  ];
  const studies = treatments.map((treatment, index) => {
    let content = brush(treatment);
    if (index === 18) content = { use: "specimen", mask: { mode: "inverted_alpha", element: content } };
    if (index === 19) {
      content = {
        children: ["linear", "kubelka_munk"].flatMap((mode, row) => [
          ...Array.from({ length: 16 }, (_, column) => rect(
            [40 + column * 27, 60 + row * 112, 67 + column * 27, 132 + row * 112],
            { pigment: { first: "#ffda28", second: "#2466cc", mode, amount: expr(`min(${column} / 15 * (0.4 + 0.6 * ${progress}), 1)`) } },
          )),
          text(["Linear light", "Pigment approximation"][row], 14, 40, 140 + row * 112),
        ])
      };
    }
    if (index === 20) {
      content = {
        children: [
          content,
          {
            ...brush({ ink: { seed: 0, height_map: 2 } }),
            filters: [{
              optics: {
                relight: {
                  height_from: "luma", light_dir: [expr(`cos(${progress} * 6)`), -0.5, 0.8],
                  color: "#ffffff", ambient: 0.35, specular: 1,
                }
              }
            }],
            blend: "multiply",
          },
        ]
      };
    }
    return { children: [rect([0, 0, 512, 304], "#ffffff"), content] };
  });
  const result = document("Ink Features", "BRUSHES", pages, studies);
  result.$schema = "../../schemas/motion.schema.json";
  result.params.ink_seed = 20261027;
  result.params.fonts = [];
  result.fonts = { $param: "fonts" };
  return result;
}

function inkJourney() {
  const p = "t / $seconds";
  const frame = "floor(t * 30)";
  const ink = "#2b1d14";
  const layers = [];
  const ridge = (index) => ({
    sample: {
      segments: 64, range: [0, 1],
      source: expr([
        "35 + t * 896",
        `357 - ${index} * 23 - exp(-pow((35 + t * 896 - 515 - ${index} * 8) / 125, 2)) * (91 + ${index} * 7) - exp(-pow((35 + t * 896 - 300) / 95, 2)) * 35 + sin((35 + t * 896) * 0.016 + ${index} * 0.4) * 9`,
      ]),
    },
  });
  const ridgePoints = (index) => Array.from({ length: 65 }, (_, sample) => {
    const x = 35 + sample * 14;
    const y = 357 - index * 23 - Math.exp(-(((x - 515 - index * 8) / 125) ** 2)) * (91 + index * 7)
      - Math.exp(-(((x - 300) / 95) ** 2)) * 35 + Math.sin(x * 0.016 + index * 0.4) * 9;
    return `${sample ? "L" : "M"}${x} ${y}`;
  }).join(" ");
  layers.push({
    rect: { bounds: [0, 0, 960, 540] },
    fill: {
      radial: {
        center: [480, 250], radius: 620, options: { units: "user_space" },
        stops: [[0, "#e6d5ae"], [0.7, "#d2bb88"], [1, "#b99a62"]].map(([offset, color]) => ({ offset, color })),
      }
    },
  });
  for (const [bounds, width, paint] of [
    [[24, 24, 936, 516], 1.2, "#2b1d148c"],
    [[30, 30, 930, 510], 0.6, "#2b1d1459"],
  ]) layers.push({ rect: { bounds }, stroke: { width, paint } });
  const ridges = [0, 2, 4].map((index) => ({
    path: { data: `${ridgePoints(index)} L945 460 L35 460 Z` },
    fill: linear([0, 120 + index * 24], [0, 420], [
      [0, `rgb(90 61 38 / ${0.42 - index * 0.06})`], [1, "#2b1d1400"],
    ]),
    opacity: expr(`clamp((${p} - 0.12 - ${index} * 0.05) * 3, 0, 1)`),
  }));
  for (let index = 0; index < 6; index += 1) {
    ridges.push({
      brush_stroke: {
        points: ridge(index), width: 6 + index * 1.1, paint: ink,
        pressure: [0.2, 1.25, 0.15], nib_aspect: 0.42, nib_mode: { fixed: degrees(-0.35) },
        ink_load: [1, 0.75], material_work_limit: 64000000,
        ink: {
          seed, dryness: 0.1, fibers: 0.75, granulation: 0.2,
          paper: { procedural: { seed: 71, strength: 0.2, scale: 1.2 } },
          edge_pooling: [0.22, 0.8],
        },
        write: expr(`clamp((${p} - ${index} * 0.075) / 0.43, 0, 1)`),
      },
    });
  }
  for (let index = 0; index < 7; index += 1) ridges.push({
    path: { data: `M${300 + index * 25} ${395 + index * 6} Q480 ${385 + index * 5} ${850 - index * 20} ${394 + index * 6}` },
    stroke: { width: 0.75, paint: "#2b1d1440" },
    opacity: expr(`clamp((${p} - 0.45) * 4, 0, 1)`),
  });
  const birds = [[560, 150, 9], [592, 132, 7], [528, 126, 6]].map(([x, y, size], index) => {
    const flap = `sin(${p} * 60 + ${index} * 1.7) * ${size} * 0.35`;
    const points = [];
    for (let wing = 0; wing < 2; wing += 1) {
      for (let sample = wing; sample <= 32; sample += 1) {
        const u = sample / 32;
        const v = 1 - u;
        const left = wing === 0
          ? x - v * v * size - 2 * v * u * size * 0.4
          : x + 2 * v * u * size * 0.4 + u * u * size;
        const edge = wing === 0 ? v * v : u * u;
        points.push(expr([
          `${left} + ${p} * 150`,
          `${y - 2 * v * u * size * 0.5} - ${p} * 24 + ${edge} * (${flap})`,
        ]));
      }
    }
    return {
      polyline: { animated: points },
      stroke: { width: 1.3, paint: "#2b1d14cc" },
      opacity: expr(`smoothstep(0.25, 0.4, ${p})`),
    };
  });
  layers.push({
    children: [
      { circle: { center: [735, 173], radius: 34 }, fill: "#b4522c", opacity: expr(`clamp(${p} * 5, 0, 1)`), blend: "multiply" },
      {
        children: ridges, pivot: { point: [480, 270] }, scale: expr(`1 + ${p} * 0.035`),
        translate: expr([`-${p} * 18 * (1 + ${p} * 0.035)`, `${p} * 6 * (1 + ${p} * 0.035)`])
      },
      ...birds,
    ],
    clip: { shape: { rect: [48, 92, 912, 442] }, mode: "geometric" },
  });
  const typed = `floor(clamp((${p} - 0.04) / 0.22 * 17, 0, 17))`;
  const caption = text("THE LONG WAY HOME", 22, 52, 52, ink, "American Typewriter");
  Object.assign(caption.text, { weight: 700, letter_spacing: 3, split: "characters", visible_parts: expr(typed) });
  layers.push(caption, {
    ...rect(expr([`52 + (${typed}) * 16.8`, "54", `63 + (${typed}) * 16.8`, "76"]), ink),
    opacity: expr(`if(mod(floor(${p} * 90), 2) == 0, if(${p} < 0.34, 0.85, 0), 0)`),
  });
  const subtitle = text("A landscape, one breath at a time.", 14, 52, 468, ink, "American Typewriter");
  subtitle.text.letter_spacing = 1;
  subtitle.opacity = expr(`smoothstep(0.3, 0.45, ${p})`);
  const stamp = `smoothstep(0.6, 0.72, ${p})`;
  layers.push(subtitle, {
    children: [
      { circle: { center: [0, 0], radius: 30 }, stroke: { width: 2.2, paint: "#b4522c" } },
      { circle: { center: [0, 0], radius: 24 }, stroke: { width: 0.8, paint: "#b4522c" } },
      {
        ...text("HOME", 12, -17, -9, "#b4522c", "American Typewriter"),
        text: { content: "HOME", family: "American Typewriter", size: 12, weight: 700, letter_spacing: 2 }
      },
    ],
    rotate: degrees(-0.22), translate: [844, 62],
    scale: expr(`1.5 - 0.5 * ${stamp}`), opacity: expr(`${stamp} * 0.8`), blend: "multiply",
  });
  const date = text("'87  10  27", 18, 770, 474, "#ff7a2f", "Menlo");
  Object.assign(date.text, { weight: 700, letter_spacing: 2 });
  date.opacity = expr(`smoothstep(0.5, 0.6, ${p})`);
  date.filters = [{ glow: { threshold: 0.2, sigma: 3, intensity: 0.9 } }];
  layers.push(date);
  // JSON film damage uses explicit frame-hashed expressions, not Rust's keyed scene RNG.
  const random = (key, clock = frame) => `fract(sin((${clock}) * 12.9898 + ${key} * 78.233 + $ink_seed) * 43758.5453)`;
  for (let index = 0; index < 22; index += 1) layers.push({
    circle: { center: expr([`${random(index * 3)} * 960`, `${random(index * 3 + 1)} * 540`]), radius: expr(`0.6 + ${random(index * 3 + 2)} * 1.8`) },
    fill: index % 4 === 0 ? "#ffffff8c" : "#2b1d148c",
  });
  const scratchX = `${random(80, `floor((${frame}) / 2)`)} * 960`;
  layers.push({
    line: { from: expr([scratchX, "0"]), to: expr([`${scratchX} + 1.5`, "540"]) },
    stroke: { width: 0.8, paint: "#ffffff59" },
    opacity: expr(`if(${random(81, `floor((${frame}) / 2)`)} > 0.62, 1, 0)`),
  });
  layers.push({
    path: { data: "M0 0 Q22 -14 40 6" }, stroke: { width: 0.7, paint: "#2b1d1499" },
    translate: expr([`${random(82, `floor((${frame}) / 3)`)} * 860 + 40`, `${random(83, `floor((${frame}) / 3)`)} * 440 + 40`]),
    opacity: expr(`if(${random(84, `floor((${frame}) / 3)`)} > 0.8, 1, 0)`),
  });
  layers.push({
    rect: { bounds: [0, 0, 960, 540] },
    fill: {
      radial: {
        center: [960, 0], radius: expr("420 + (0.5 + 0.5 * sin(t * 0.9)) * 80"),
        options: { units: "user_space" },
        stops: [[0, "#ff9a4a8c"], [0.5, "#ff5a2a29"], [1, "#ff5a2a00"]].map(([offset, color]) => ({ offset, color })),
      }
    }, blend: "screen",
  });
  return {
    $schema: "../../schemas/motion.schema.json", version: 1, name: "The Long Way Home",
    size: [960, 540], fps: 30, duration: { $param: "seconds" },
    params: { seconds: 8, seed: 20261027, ink_seed: 20261027,
      fonts: [] },
    fonts: { $param: "fonts" },
    tracks: [{
      items: [{
        scene: {
          duration: { $param: "seconds" }, name: "Ink journey",
          content: {
            children: layers, clip: { shape: { rect: [0, 0, 960, 540] }, mode: "geometric" },
            filters: [
              { color_adjust: { exposure: expr(`(${random(90)} - 0.5) * 0.1`), contrast: 0.94, saturation: 0.82, hue: 0 } },
              { optics: { lens_vignette: { angle_of_view: 75, strength: 0.6, roundness: 0 } } },
              { grain: { seed: { $param: "seed" }, strength: 0.05, frame: expr(frame) } },
            ],
          },
        }
      }]
    }],
  };
}

module.exports = { inkFeatures, inkJourney };
