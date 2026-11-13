#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const particleFeatures = require("./particle-features.cjs");

const assets = path.resolve(__dirname, "../gallery/assets");
const template = JSON.parse(
  fs.readFileSync(path.join(assets, "compositing-atlas.json"), "utf8"),
);
const expression = (expr) => ({ expr });
const progress = "@clock.rotate";
const amount = `(0.25 + 0.75 * ${progress})`;
const copy = (value) => structuredClone(value);
const rect = (bounds, fill) => ({ rect: { bounds }, fill });
const text = (content, size, left, top) => ({
  text: { content, family: "", size, weight: 500, letter_spacing: 0 },
  fill: "#b9cec5",
  translate: [left, top],
});
const options = { units: "user_space" };
const stop = (offset, color) => ({ offset, color });
const linear = (start, end, stops, extra = {}) => ({
  linear: { start, end, stops, options: { ...options, ...extra } },
});
const pair = (
  left,
  right,
  leftLabel = "Original",
  rightLabel = "Processed",
) => ({
  children: [
    { use: "divider" },
    { children: [left], scale: 0.48, translate: [-128, 0] },
    { children: [right], scale: 0.48, translate: [128, 0] },
    { use: "left_label", text: { use: "pair_label", content: leftLabel } },
    { use: "right_label", text: { use: "pair_label", content: rightLabel } },
  ],
});

const filterPages = [
  [
    "Blur",
    "Gaussian-style blur spreads",
    "premultiplied color across",
    "transparent source borders.",
  ],
  [
    "Drop shadow",
    "A blurred alpha silhouette",
    "is offset behind the source;",
    "the original remains sharp.",
  ],
  [
    "Glow",
    "Bright source pixels add",
    "a single-scale light halo",
    "without replacing the source.",
  ],
  [
    "Color adjustment",
    "Exposure, contrast and hue",
    "operate on linear color;",
    "alpha remains unchanged.",
  ],
  [
    "Bloom",
    "Soft highlight extraction",
    "feeds several blur octaves",
    "for a broad light envelope.",
  ],
  [
    "Halation",
    "Bright edges receive a warm",
    "red-orange halo assembled",
    "from ordinary primitives.",
  ],
  [
    "Anamorphic",
    "A bright source produces",
    "horizontal tinted streaks",
    "with a narrow vertical blur.",
  ],
  [
    "Lens distortion",
    "Radial sampling bends the",
    "source around its center;",
    "auto-fit protects corners.",
  ],
  [
    "Chromatic dispersion",
    "Color channels sample at",
    "different radial distances",
    "to separate bright edges.",
  ],
  [
    "Lens vignette",
    "Cosine-fourth falloff",
    "darkens the outer field",
    "around the optical center.",
  ],
  [
    "Aperture bokeh",
    "A six-blade aperture shapes",
    "defocused highlights with",
    "bounded multi-tap sampling.",
  ],
  [
    "Radial blur",
    "Samples travel toward an",
    "explicit light position",
    "with decaying ray weights.",
  ],
  [
    "Lens flare",
    "Mirrored highlight ghosts",
    "and a radial halo share",
    "an explicit optical center.",
  ],
  [
    "Relight",
    "Luminance becomes height;",
    "surface normals respond to",
    "a moving light direction.",
  ],
  [
    "Seeded grain",
    "Scene-linear monochrome",
    "noise has explicit seeds",
    "and preserves source alpha.",
  ],
  [
    "Directional blur",
    "Independent axis sigmas",
    "stretch a custom blur",
    "along the horizontal axis.",
  ],
  [
    "Filter order",
    "Exposure before a threshold",
    "changes extracted energy;",
    "reversing the chain differs.",
  ],
];

const filters = [
  { blur: { sigma: expression(`12 * ${amount}`) } },
  {
    drop_shadow: {
      offset: [18, 20],
      sigma: expression(`9 * ${amount}`),
      color: "#ffc777",
    },
  },
  {
    glow: {
      threshold: 0.5,
      sigma: 14,
      intensity: expression(`1.8 * ${amount}`),
    },
  },
  {
    color_adjust: {
      exposure: 0.5,
      contrast: 1.2,
      saturation: expression(`0.4 + ${progress}`),
      hue: expression(`90 * ${progress}`),
    },
  },
  {
    bloom: {
      threshold: 0.6,
      knee: 0.2,
      radius: 5,
      levels: 3,
      falloff: 0.55,
      intensity: expression(`1.5 * ${amount}`),
      tint: "#ffffff",
    },
  },
  { halation: { strength: expression(`2 * ${amount}`), radius: 12 } },
  {
    optics: {
      anamorphic: {
        threshold: 0.6,
        length: expression(`32 * ${amount}`),
        stretch: 12,
        tint: "#8bd5f3",
        intensity: 1.5,
      },
    },
  },
  {
    optics: {
      lens: {
        distortion: expression(`0.65 * ${amount}`),
        chromatic: 0,
        auto_fit: true,
      },
    },
  },
  {
    optics: {
      lens: {
        distortion: 0,
        chromatic: expression(`0.07 * ${amount}`),
        auto_fit: true,
      },
    },
  },
  {
    optics: {
      lens_vignette: {
        angle_of_view: 125,
        strength: expression(amount),
        roundness: 0.5,
      },
    },
  },
  {
    optics: {
      bokeh: {
        radius: expression(`14 * ${amount}`),
        shape: {
          polygon: { sides: 6, rotation: expression(`degrees(${progress})`) },
        },
        highlight_boost: 1.2,
      },
    },
  },
  {
    optics: {
      radial_blur: {
        center: [220, 100],
        strength: expression(`0.5 * ${amount}`),
        samples: 16,
        decay: 0.92,
      },
    },
  },
  {
    optics: {
      lens_flare: {
        center: [300, 185],
        ghosts: 4,
        threshold: 0.6,
        intensity: expression(`2 * ${amount}`),
        tint: "#ffc777",
      },
    },
  },
  {
    optics: {
      relight: {
        height_from: "luma",
        light_dir: [
          expression(`cos(${progress} * 6)`),
          expression(`sin(${progress} * 6)`),
          0.7,
        ],
        color: "#ffc777",
        ambient: 0.25,
        specular: 1.5,
      },
    },
  },
  {
    grain: {
      seed: { $param: "seed" },
      strength: expression(`0.12 * ${amount}`),
    },
  },
  { blur_xy: { sigma_x: expression(`22 * ${amount}`), sigma_y: 1 } },
];
const exposure = {
  color_adjust: {
    exposure: expression(`1.5 * ${amount}`),
    contrast: 1,
    saturation: 1,
    hue: 0,
  },
};
const glow = { glow: { threshold: 0.9, sigma: 18, intensity: 1 } };
const filterStudies = filters.map((filter) =>
  pair({ use: "specimen" }, { use: "specimen", filters: [filter] }),
);
filterStudies.push(
  pair(
    { use: "specimen", filters: [glow, exposure] },
    { use: "specimen", filters: [exposure, glow] },
    "Glow / exposure",
    "Exposure / glow",
  ),
);

const vectorPages = [
  [
    "Arc-length trim",
    "A cubic path is revealed",
    "by normalized arc distance",
    "rather than control-point time.",
  ],
  [
    "Dash and phase",
    "Explicit dash intervals",
    "move along the same curve",
    "with a changing phase.",
  ],
  [
    "Stroke caps",
    "Butt, round and square",
    "caps produce different ends",
    "for the same centerline.",
  ],
  [
    "Stroke joins",
    "Miter, round and bevel",
    "joins resolve corners with",
    "different outline geometry.",
  ],
  [
    "Fill rules",
    "Same-winding nested paths",
    "compare NonZero with",
    "EvenOdd hole formation.",
  ],
  [
    "Linear gradient",
    "Color stops interpolate",
    "along an explicit moving",
    "line in local coordinates.",
  ],
  [
    "Focal radial",
    "A displaced inner focus",
    "moves the highlight inside",
    "a bounded radial gradient.",
  ],
  [
    "Sweep gradient",
    "Angular color intervals",
    "rotate around an explicit",
    "center through a full turn.",
  ],
  [
    "Gradient extension",
    "Pad, repeat and reflect",
    "extend the same short",
    "gradient interval differently.",
  ],
  [
    "Interpolation space",
    "The same encoded stops",
    "compare linear-light and",
    "sRGB color interpolation.",
  ],
  [
    "SVG vector tree",
    "Original SVG geometry",
    "stays vector as the mark",
    "rotates and scales.",
  ],
  [
    "Vector morph",
    "Matching vector identities",
    "interpolate geometry and",
    "paint between two marks.",
  ],
  [
    "Vector writing",
    "Cumulative contour length",
    "draws temporary outlines",
    "then hands off to the fill.",
  ],
];
const source = {
  data: '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="304"><circle id="mark" cx="256" cy="152" r="92" fill="#66e0c2"/><path id="line" d="M190 150 L240 195 L322 104" fill="none" stroke="#17231f" stroke-width="12"/></svg>',
};
const target = {
  data: '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="304"><path id="mark" d="M256 44 L376 152 L256 260 L136 152 Z" fill="#ffc777"/><path id="line" d="M190 190 L256 112 L322 190" fill="none" stroke="#17231f" stroke-width="12"/></svg>',
};
const curve = {
  path: { data: "M48 212 C140 -20 350 330 464 80" },
  stroke: { width: 7, paint: "#66e0c2" },
};
const nested =
  "M176 72 L336 72 L336 232 L176 232 Z M216 112 L296 112 L296 192 L216 192 Z";
const strokeStyles = (page) => ({
  children: [0, 1, 2].flatMap((index) => {
    const left = 40 + index * 160;
    const names =
      page === 2 ? ["Butt", "Round", "Square"] : ["Miter", "Round", "Bevel"];
    return [
      {
        polyline: {
          animated: [
            [left, 198],
            expression([String(left + 60), `96 + ${progress} * 30`]),
            [left + 120, 198],
          ],
        },
        stroke: {
          width: 22,
          paint: "#8bd5f3",
          cap: page === 2 ? ["butt", "round", "square"][index] : "butt",
          join: page === 3 ? [{ miter: { limit: 6 } }, "round", "bevel"][index] : "round",
        },
      },
      text(names[index], 15, left + 35, 244),
    ];
  }),
});
const vectorStudies = [
  {
    ...copy(curve),
    trim: { start: 0, end: expression(`0.1 + 0.9 * ${progress}`) },
  },
  {
    ...copy(curve),
    stroke: {
      ...curve.stroke,
      dash: [18, 10, 3, 10],
      dash_offset: expression(`${progress} * 100`),
    },
  },
  strokeStyles(2),
  strokeStyles(3),
  {
    children: [
      {
        path: { data: nested },
        fill: "#66e0c2",
        fill_rule: "non_zero",
        translate: [-120, 0],
      },
      {
        path: { data: nested },
        fill: "#ffc777",
        fill_rule: "even_odd",
        translate: [120, 0],
      },
    ],
    rotate: expression(`degrees((${progress} - 0.5) * 0.25)`),
  },
  rect(
    [40, 52, 472, 252],
    linear([40, 52], expression([`260 + ${progress} * 210`, "252"]), [
      stop(0, "#66e0c2"),
      stop(0.5, "#8bd5f3"),
      stop(1, "#d64b36"),
    ]),
  ),
  rect([40, 52, 472, 252], {
    radial: {
      center: [256, 152],
      radius: 210,
      focal: expression([`170 + ${progress} * 150`, "120"]),
      stops: [stop(0, "#ffc777"), stop(1, "#3564c4")],
      options,
    },
  }),
  rect([40, 52, 472, 252], {
    sweep: {
      center: [256, 152],
      start_angle: expression(`degrees(${progress} * 6)`),
      end_angle: expression(`degrees(${progress} * 6 + tau)`),
      stops: [stop(0, "#66e0c2"), stop(0.5, "#ffc777"), stop(1, "#66e0c2")],
      options,
    },
  }),
  {
    children: ["pad", "repeat", "reflect"].flatMap((extend, index) => {
      const top = 40 + index * 80;
      return [
        rect(
          [36, top, 476, top + 44],
          linear(
            expression([`160 + ${progress} * 40`, "0"]),
            expression([`280 + ${progress} * 40`, "0"]),
            [stop(0, "#8bd5f3"), stop(1, "#ffc777")],
            { extend },
          ),
        ),
        text(["Pad", "Repeat", "Reflect"][index], 13, 36, top + 48),
      ];
    }),
  },
  {
    children: ["linear_rgb", "srgb"].flatMap((space, index) => {
      const top = 65 + index * 115;
      return [
        rect(
          [40, top, 472, top + 65],
          linear(
            [40, 0],
            expression([`400 + ${progress} * 60`, "0"]),
            [stop(0, "#d64b36"), stop(1, "#3564c4")],
            { space },
          ),
        ),
        text(["Linear RGB", "sRGB"][index], 14, 40, top + 70),
      ];
    }),
  },
  {
    svg: { source },
    rotate: expression(`degrees((${progress} - 0.5) * 0.9)`),
    scale: expression(`0.8 + 0.2 * ${progress}`),
  },
  {
    svg_morph: {
      from: source,
      to: target,
      progress: expression(
        `if(${progress} < 0.5, 4 * ${progress} * ${progress} * ${progress}, 1 - pow(-2 * ${progress} + 2, 3) / 2)`,
      ),
    },
  },
  {
    svg: {
      source,
      write: expression(progress),
      write_options: { width: 3, outline_fraction: 0.8 },
    },
  },
];

const particlePages = [
  [
    "Sparks",
    "A ballistic burst with",
    "tapered HDR trails and",
    "cooling blackbody color.",
  ],
  [
    "Embers",
    "Warm embers rise through",
    "a curl-noise field, cooling",
    "as their glow fades.",
  ],
  [
    "Dust",
    "Slow dust drifts through",
    "a depth field with small,",
    "hexagonal bokeh.",
  ],
  [
    "Motes",
    "Soft floating motes use",
    "larger bokeh and subtle",
    "depth-driven parallax.",
  ],
  [
    "Snow",
    "Three depth layers of snow",
    "fall through a gentle wind",
    "with drifting parallax.",
  ],
  [
    "Rain",
    "Stretched drops strike",
    "the floor and trigger",
    "short-lived splash bursts.",
  ],
  [
    "Smoke",
    "Procedural smoke sprites",
    "expand, curl and fade",
    "as they rise from below.",
  ],
  [
    "Fire",
    "Buoyant fire cores rise",
    "and shrink, cooling from",
    "white heat toward red.",
  ],
  [
    "Confetti",
    "Colored quads tumble",
    "under gravity and drag,",
    "with seeded flip phases.",
  ],
  [
    "Fireworks",
    "A rocket bursts on death,",
    "then its sparks trigger",
    "smaller secondary bursts.",
  ],
  [
    "Fireflies",
    "Seeded drift carries",
    "warm lights with gently",
    "breathing intensity.",
  ],
  [
    "Logo Dissolve",
    "A colored vector surface",
    "breaks into particles that",
    "keep their source colors.",
  ],
  [
    "Logo Assemble",
    "Scattered colored points",
    "ease back into their",
    "original vector surface.",
  ],
];
const particleTags = [
  "BURST / TRAILS",
  "CURL / TEMPERATURE",
  "DRIFT / BOKEH",
  "DEPTH / PARALLAX",
  "WIND / DEPTH",
  "COLLISION / SPLASH",
  "SPRITES / CURL",
  "SPRITES / TEMPERATURE",
  "GRAVITY / FLIP",
  "DEATH / SUB-EMITTERS",
  "DRIFT / INTENSITY",
  "VECTOR / DISPERSAL",
  "VECTOR / CONVERGENCE",
];
const particleStudies = particlePages.map(([name], index) => {
  const [offset, scale] =
    index === 0
      ? [0.12, 1.35]
      : index === 9
        ? [0.1, 1.6]
        : index >= 11
          ? [0.04, 3.9]
          : [0.2, 3.5];
  const preset = {
    kind: name.toLowerCase().replaceAll(" ", "_"),
    bounds: [0, 0, 512, 304],
    density: { $calc: "$count / 72" },
  };
  if (index >= 11)
    preset.logo = [
      {
        data: "M179.2 91.2 L332.8 91.2 L332.8 212.79999999999998 L179.2 212.79999999999998 Z",
        fill: {
          linear: {
            start: [0, 0],
            end: [1, 0],
            stops: [stop(0, "#25bb90"), stop(1, "#ff4060")],
          },
        },
      },
    ];
  const study = {
    particles: {
      seed: { $param: "seed" },
      duration: 4,
      preset,
      sample_time: expression(`${offset} + ${progress} * ${scale}`),
    },
  };
  if (index === 6 || index === 7) {
    const zoom = index === 6 ? 2 : 4;
    study.pivot = { point: [0, 0] };
    study.scale = zoom;
    study.translate = [512 * (1 - zoom) * 0.5, 264 - 304 * zoom];
  }
  if ([0, 1, 7, 9, 10].includes(index))
    study.filters = [
      {
        bloom: {
          threshold: 0.4,
          knee: 0.2,
          radius: 4,
          levels: 3,
          falloff: 0.6,
          intensity: 0.8,
          tint: "#ffffff",
        },
      },
    ];
  return study;
});

function particleDocument() {
  const result = document(
    "Particle Atlas",
    "PARTICLES",
    particlePages,
    particleStudies,
  );
  result.params.count = 72;
  result.tracks[0].items.forEach(({ scene }, index) => {
    const elements = scene.content.children;
    const captions = elements.find(
      (element) => element.name === "Page captions",
    ).children[0].children;
    captions.push({
      text: {
        content: particleTags[index],
        family: "",
        size: 11,
        weight: 600,
        letter_spacing: 1.2,
      },
      fill: "#56635c",
      translate: [48, 288],
    });
    if (index === 8) {
      const viewport = elements.find((element) => element.clip);
      for (const element of viewport.children) {
        if (element.fill === "#17231f") element.fill = "#e7eeea";
        else if (element.fill === "#26372f") element.fill = "#d5dfd9";
      }
    }
  });
  return result;
}

function document(title, category, pages, studies) {
  assert.equal(pages.length, studies.length);
  const result = copy(template);
  result.params = {
    seconds: 8,
    seed: 20261027,
    title,
    pages: pages.length,
    segment_width: { $calc: "(864 - 4 * ($pages - 1)) / $pages" },
  };
  const definitions = [
    "heading",
    "body",
    "next",
    "page_number",
    "heading_node",
    "line_one",
    "line_two",
    "line_three",
    "next_node",
    "number_node",
    "specimen",
    "pair_label",
    "divider",
    "left_label",
    "right_label",
  ];
  result.defs = Object.fromEntries(
    definitions.map((name) => [name, copy(template.defs[name])]),
  );
  const scene = result.tracks[0].items[0].scene;
  scene.name = `${title} pages`;
  const children = scene.content.children;
  children.find(
    (element) => element.text?.content === "MOTION GALLERY  /  RENDERING",
  ).text.content = `MOTION GALLERY  /  ${category}`;
  children.find((element) => element.text?.content === "/ 26").text.content =
    `/ ${pages.length}`;
  children.find((element) => element.name === "Page captions").children =
    pages.map(([name, ...lines], index) => ({
      name,
      opacity: expression(`if(@clock.translate.y == ${index}, 1, 0)`),
      children: [
        {
          use: "heading_node",
          text: {
            use: "heading",
            content: name,
            size: name.length > 16 ? 22 : 26,
          },
        },
        {
          use: "number_node",
          text: {
            use: "page_number",
            content: String(index + 1).padStart(2, "0"),
          },
        },
        ...lines.map((content, row) => ({
          use: ["line_one", "line_two", "line_three"][row],
          text: { use: "body", content },
        })),
        {
          use: "next_node",
          text: {
            use: "next",
            content:
              index + 1 < pages.length
                ? `Next  /  ${pages[index + 1][0]}`
                : "End of atlas",
          },
        },
      ],
    }));
  const viewport = children.find((element) => element.clip);
  const stage = viewport.children.filter(
    (element) => element.rect && ["#17231f", "#26372f"].includes(element.fill),
  );
  assert.equal(stage.length, 12);
  viewport.children = [
    ...stage,
    ...studies.map((study, index) => ({
      children: [study],
      opacity: expression(`if(@clock.translate.y == ${index}, 1, 0)`),
    })),
  ];
  children.find((element) => element.name === "Footer").children =
    pages.flatMap((_, index) => {
      const left = `48 + ${index} * ($segment_width + 4)`;
      const top = `494 - if(@clock.translate.y == ${index}, 2, 1)`;
      const bottom = `494 + if(@clock.translate.y == ${index}, 2, 1)`;
      return [
        rect(
          expression([left, top, `${left} + $segment_width`, bottom]),
          "#c3cbc5",
        ),
        rect(
          expression([
            left,
            top,
            `${left} + $segment_width * clamp(@clock.translate.x * $pages - ${index}, 0, 1)`,
            bottom,
          ]),
          "#d64b36",
        ),
      ];
    });
  result.tracks[0].items = pages.map(([name], index) => {
    const start = `page_${index}_start`;
    const end = `page_${index}_end`;
    result.params[start] = {
      $calc: `ceil($seconds * 30 * ${index} / $pages) / 30`,
    };
    result.params[end] = {
      $calc: `ceil($seconds * 30 * ${index + 1} / $pages) / 30`,
    };
    const page = copy(scene);
    page.name = name;
    page.duration = { $calc: `$${end} - $${start}` };
    const elements = page.content.children;
    const clock = elements.find((element) => element.id === "clock");
    clock.translate = expression([`(t + $${start}) / $seconds`, String(index)]);
    clock.rotate = expression(
      `clamp((t + $${start}) / $seconds * $pages - ${index}, 0, 1)`,
    );
    const captions = elements.find(
      (element) => element.name === "Page captions",
    );
    const caption = captions.children[index];
    delete caption.opacity;
    captions.children = [caption];
    elements.find((element) => element.clip).children = [
      ...copy(stage),
      copy(studies[index]),
    ];
    return { scene: page };
  });
  return result;
}

assert(
  process.argv.slice(2).every((argument) => argument === "--check"),
  "Only --check is supported",
);
for (const [filename, value] of [
  [
    "filter-atlas.json",
    document("Filter Atlas", "RENDERING", filterPages, filterStudies),
  ],
  [
    "vector-atlas.json",
    document("Vector Atlas", "VECTOR", vectorPages, vectorStudies),
  ],
  ["particle-atlas.json", particleDocument()],
  ["particle-features.json", particleFeatures(document)],
]) {
  const destination = path.join(assets, filename);
  const serialized = `${JSON.stringify(value, null, 2)}\n`;
  if (process.argv.includes("--check"))
    assert.equal(
      fs.readFileSync(destination, "utf8"),
      serialized,
      `${filename} is stale`,
    );
  else fs.writeFileSync(destination, serialized);
  console.log(
    `${filename}: ${value.params.pages} pages ${process.argv.includes("--check") ? "verified" : "generated"}`,
  );
}
