"use strict";

const calc = (source) => ({ $calc: source });
const expr = (source) => ({ expr: source });
const seed = { $param: "seed" };
// Curl fields take 32-bit seeds; the gallery passes the low bits of its seed.
const curlSeed = { $param: "curl_seed" };
const density = "$count * 2";
const color = [0.3, 0.85, 0.65, 1];
const response = { restitution: 0.75, friction: 0.1, kill_on_hit: false };
const rect = (bounds, fill) => ({ rect: { bounds }, fill });
const circle = (center, radius, fill) => ({ circle: { center, radius }, fill });
const text = (content, size, left, top) => ({
  text: { content, family: "", size, weight: 500, letter_spacing: 0 },
  fill: "#b4c8bd",
  translate: [left, top],
});
const group = (...children) => ({ children });
const operator = (kind, value) => ({ operator: { [kind]: value } });
const curve = (start, end) => ({
  stops: [
    [0, start],
    [1, end],
  ],
  easing: "linear",
});
const birth = (shape, rate, speed, lifetime, tint = color, extra = {}) => ({
  ...shape,
  ...(rate === null ? {} : { rate: calc(rate) }),
  speed,
  lifetime,
  ...(tint === null ? {} : { premultiplied_color: tint }),
  ...extra,
});
const burst = (count, center, speed, lifetime = 4, tint = color, extra = {}) =>
  birth(
    {
      burst: {
        time: 0,
        count: typeof count === "string" ? calc(count) : count,
        position: center,
      },
    },
    null,
    speed,
    lifetime,
    tint,
    extra,
  );
const dots = (radius = 2.4) => ({ dots: { radius } });
const simulation = (
  emitters,
  modifiers = [],
  style = dots(),
  configuration = {},
  extra = {},
) => ({
  particles: {
    seed,
    duration: 4,
    sample_time: expr("0.25 + @clock.rotate * 3.4"),
    emitters,
    modifiers,
    style,
    configuration: { max_work: 1600000000, ...configuration },
    ...extra,
  },
});
const cloud = () =>
  birth(
    { disk: { center: [256, 152], radius: 102 } },
    density,
    12,
    3.8,
    [0.25, 0.85, 0.65, 1],
  );
const trajectory = (model, style) => ({
  trajectory: {
    model,
    style,
    sample_time: expr("0.25 + @clock.rotate * 3.4"),
  },
});
const points = (population, horizontal, vertical) => ({
  sample: {
    source: expr([horizontal, vertical]),
    range: [0, calc(`${population} - 1`)],
    segments: calc(`${population} - 1`),
  },
});
const ring = (population, radius) =>
  points(
    population,
    `256 + cos(t / (${population}) * tau) * ${radius}`,
    `152 + sin(t / (${population}) * tau) * ${radius}`,
  );
const surface = {
  bounds: [0, 0, 512, 304],
  inherit_color: true,
  paths: [
    {
      data: "M179.2 91.2 L332.8 91.2 L332.8 212.79999999999998 L179.2 212.79999999999998 Z",
      fill: {
        linear: {
          start: [0, 0],
          end: [1, 0],
          stops: [
            { offset: 0, color: "#25bb90" },
            { offset: 1, color: "#ff4060" },
          ],
        },
      },
    },
  ],
};
const phase = () => [
  operator("custom_slot", { slot: 0, modulus: 5, scale: 1 }),
  operator("custom_slot", { slot: 2, modulus: null, scale: 0.7 }),
];
const curl = (frequency, strength, phase_speed) => ({
  curl_noise: { frequency, strength, phase_speed, seed: curlSeed },
});
const streaming = () =>
  birth(
    { point: [100, 208] },
    `${density} * 0.5`,
    22,
    3.2,
    [0.3, 0.85, 0.7, 1],
    { velocity: [80, -26] },
  );
const falling = (rate) =>
  birth({ rect: [112, 16, 400, 18] }, rate, 4, 3.8, [0.3, 0.8, 1, 1], {
    velocity: [0, 72],
    size: 2,
  });
const materials = [
  { material: "soft_circle", size: 40 },
  { material: { smoke: { scale: 3, evolution: 0 } }, size: 40 },
  { material: "star", size: 32 },
  { material: "ring", size: 40 },
  { material: "fire_core", size: 32 },
];
const sprites = (size, mode, frames = materials, padded = true) => ({
  sprites: { seed, size, mode, frames, padded },
});
const spriteBirth = () =>
  birth(
    { rect: [96, 72, 416, 232] },
    `${density} / 3`,
    10,
    3.8,
    [0.6, 0.85, 1, 1],
  );
const timeBirth = () =>
  birth({ point: [84, 152] }, density, 8, 3.5, color, { velocity: [82, 0] });
const colors = [
  [0.1, 0.8, 1, 1],
  [1, 0.3, 0.1, 1],
];

const pages = [
  [
    "Point emitter",
    "EMISSION / POINT",
    "Particles share an origin",
    "but receive independently",
    "seeded launch directions.",
  ],
  [
    "Disk emitter",
    "EMISSION / AREA",
    "Births cover the disk area",
    "uniformly, rather than",
    "just its circumference.",
  ],
  [
    "Rect emitter",
    "EMISSION / AREA",
    "A rectangular source",
    "distributes births across",
    "its full local surface.",
  ],
  [
    "Timed burst",
    "EMISSION / BURST",
    "One bounded population",
    "launches at an explicit",
    "time on the fixed grid.",
  ],
  [
    "Path emitter",
    "EMISSION / PATH",
    "An animated arc-length",
    "source launches particles",
    "along the curve tangent.",
  ],
  [
    "Vector surface",
    "EMISSION / COLOR",
    "Area-uniform births keep",
    "the vector surface colors",
    "without altering density.",
  ],
  [
    "Image alpha",
    "EMISSION / IMAGE",
    "Image alpha controls",
    "where particles are born;",
    "pixel colors are inherited.",
  ],
  [
    "Image luminance",
    "EMISSION / IMAGE",
    "Bright source pixels emit",
    "more particles than dark",
    "pixels in the same image.",
  ],
  [
    "Text surface",
    "EMISSION / GLYPHS",
    "Shaped glyph outlines",
    "become filled emission",
    "surfaces, not bitmaps.",
  ],
  [
    "Ballistic",
    "ANALYTIC / GRAVITY",
    "Gravity and linear drag",
    "are evaluated together",
    "at any requested time.",
  ],
  [
    "Converge",
    "ANALYTIC / TARGETS",
    "A complete point set",
    "eases from a grid into",
    "a circular target shape.",
  ],
  [
    "Orbit",
    "ANALYTIC / PHASE",
    "Seeded orbital phases",
    "provide exact positions",
    "and tangent velocities.",
  ],
  [
    "Noise drift",
    "ANALYTIC / NOISE",
    "Smooth noise displaces",
    "each stable particle ID",
    "without stepping history.",
  ],
  [
    "Gravity / wind",
    "FORCES / DRAG",
    "Gravity bends the flight;",
    "wind pushes sideways",
    "while drag damps speed.",
  ],
  [
    "Attractor",
    "FORCES / DISTANCE",
    "A softened inverse-square",
    "field pulls particles",
    "toward a shared center.",
  ],
  [
    "Radial falloff",
    "FORCES / FALLOFF",
    "Constant, linear, inverse",
    "square and custom curves",
    "control attraction range.",
  ],
  [
    "Vortex",
    "FORCES / TANGENT",
    "Tangential acceleration",
    "turns a seeded cloud",
    "around a softened center.",
  ],
  [
    "Seek targets",
    "FORCES / TARGETS",
    "Stable particle IDs seek",
    "matching targets using",
    "velocity correction.",
  ],
  [
    "Curl noise",
    "FORCES / PERLIN",
    "An evolving curl field",
    "adds divergence-free",
    "seeded accelerations.",
  ],
  [
    "Flow fields",
    "FORCES / IMAGE",
    "An image vector map adds",
    "flow to particles while",
    "a curl field evolves.",
  ],
  [
    "Boids",
    "GROUP / FLOCKING",
    "Neighbors separate, align",
    "and cohere using a stable",
    "population snapshot.",
  ],
  [
    "Spring chain",
    "GROUP / HOOKE",
    "Consecutive stable IDs",
    "are coupled by springs",
    "with explicit damping.",
  ],
  [
    "Particle contacts",
    "GROUP / COLLISION",
    "Equal-mass particles",
    "exchange impulses and",
    "separate on contact.",
  ],
  [
    "Bounce box",
    "CONTACT / BOUNDS",
    "Predicted positions",
    "reflect inside a bounded",
    "local rectangle.",
  ],
  [
    "Circle collider",
    "CONTACT / SWEPT",
    "Falling particles strike",
    "a solid circular obstacle",
    "with finite contact size.",
  ],
  [
    "Polygon collider",
    "CONTACT / SWEPT",
    "A prepared polygon",
    "deflects incoming discs",
    "along its edge normals.",
  ],
  [
    "SDF collider",
    "CONTACT / GRID",
    "A bounded distance grid",
    "and exact edge guards",
    "resolve surface contacts.",
  ],
  [
    "Collision events",
    "EVENT / SUB-EMITTER",
    "Floor contacts trigger",
    "small inherited bursts",
    "without recursive rules.",
  ],
  [
    "Lifecycle events",
    "EVENT / STAGES",
    "Birth, age and death",
    "trigger distinct colored",
    "bursts from root particles.",
  ],
  [
    "Shape gallery",
    "STYLE / GEOMETRY",
    "Polygons, rings, capsules,",
    "soft discs and quads need",
    "no image textures.",
  ],
  [
    "Historical trails",
    "STYLE / HISTORY",
    "Shared historical samples",
    "form tapered, faded and",
    "smoothed trail segments.",
  ],
  [
    "Ribbon trails",
    "STYLE / RIBBON",
    "Continuous flat-ended",
    "strips join smoothed",
    "historical positions.",
  ],
  [
    "Plexus",
    "STYLE / NEIGHBORS",
    "Stable nearby pairs",
    "form a moving network",
    "with bounded link work.",
  ],
  [
    "Bokeh / depth",
    "STYLE / APERTURE",
    "Depth changes defocus",
    "and parallax with stable",
    "far-to-near sorting.",
  ],
  [
    "Blend modes",
    "STYLE / COMPOSITE",
    "SourceOver, Plus, Screen",
    "and Multiply combine",
    "the same colored dots.",
  ],
  [
    "Sprite atlas",
    "STYLE / MATERIALS",
    "Five generated materials",
    "share a padded atlas",
    "with seeded selection.",
  ],
  [
    "Sprite sheets",
    "STYLE / FRAME SELECT",
    "Lifetime, FPS, random",
    "and custom state select",
    "frames without a cursor.",
  ],
  [
    "Evolving smoke",
    "STYLE / FLIPBOOK",
    "Explicit-time smoke",
    "materials form a sampled",
    "animated sprite sheet.",
  ],
  [
    "Stretch / flip",
    "STYLE / VELOCITY",
    "Speed stretches capsules;",
    "age and seeded phases",
    "flip falling quads.",
  ],
  [
    "Lifetime curves",
    "APPEARANCE / AGE",
    "Size, eased color and",
    "HDR intensity follow",
    "normalized particle age.",
  ],
  [
    "Color / heat",
    "APPEARANCE / FIELD",
    "Position and speed sample",
    "color fields; temperature",
    "maps Kelvin to linear RGB.",
  ],
  [
    "Custom style",
    "STYLE / CALLBACK",
    "A Rust callback converts",
    "particle state into",
    "ordinary animated nodes.",
  ],
  [
    "Prewarm",
    "TIME / LEAD-IN",
    "A prepared lead-in fills",
    "the source before the",
    "visible clip begins.",
  ],
  [
    "Loop period",
    "TIME / TWO PHASES",
    "Offset populations use",
    "crossfades to hide the",
    "wrap of a finite source.",
  ],
  [
    "Time remap",
    "TIME / SLOW MOTION",
    "An explicit query scale",
    "slows the source without",
    "changing its fixed step.",
  ],
  [
    "Reverse queries",
    "TIME / REPLAY",
    "Negative time mapping",
    "revives historical states",
    "with explicit opt-in.",
  ],
  [
    "Workers / replay",
    "EXECUTION / PARITY",
    "One and four workers",
    "produce matching states",
    "shown side by side.",
  ],
];

module.exports = function particleFeatures(document) {
  const image = { size: [48, 48], rgba: [] };
  for (let row = 0; row < 48; row++)
    for (let column = 0; column < 48; column++) {
      const horizontal = (column + 0.5) / 48;
      const vertical = (row + 0.5) / 48;
      image.rgba.push(
        Math.trunc(horizontal * 255),
        Math.trunc(vertical * 200),
        Math.trunc((1 - horizontal) * 255),
        Math.hypot(horizontal * 2 - 1, vertical * 2 - 1) < 0.92 ? 255 : 0,
      );
    }
  const studies = [
    simulation([
      birth({ point: [256, 152] }, density, 32, 3, [0.15, 0.85, 0.65, 1]),
    ]),
    group(
      circle([256, 152], 72, "#263e35"),
      simulation([
        birth(
          { disk: { center: [256, 152], radius: 72 } },
          density,
          8,
          3,
          [0.3, 0.65, 1, 1],
        ),
      ]),
    ),
    group(
      rect([96, 72, 416, 232], "#263e35"),
      simulation([
        birth({ rect: [96, 72, 416, 232] }, density, 4, 3, [1, 0.55, 0.25, 1]),
      ]),
    ),
    simulation([
      burst(`${density} * 2`, [256, 152], 34, 4, [1, 0.65, 0.25, 1]),
    ]),
    simulation([
      birth(
        { path: "M60 210 C150 -20 360 324 452 90" },
        `${density} * 2`,
        28,
        2.4,
        [0.2, 0.9, 0.65, 1],
        { path_progress: { tween: { from: 0, to: 1, duration: 4 } } },
      ),
    ]),
    simulation([birth({ surface }, `${density} * 2`, 16, 3, null)]),
    ...[false, true].map((luminance) =>
      simulation([
        birth(
          { image: { image, bounds: [140, 36, 372, 268], luminance } },
          `${density} * 3`,
          6,
          3,
          null,
        ),
      ]),
    ),
    {
      ...simulation([
        birth(
          {
            text: { content: "motion", family: "", size: 76, fill: "#54dfb2" },
          },
          `${density} * 4`,
          4,
          3,
          null,
        ),
      ]),
      translate: [108, 100],
    },
    trajectory(
      {
        ballistic: {
          center: [256, 116],
          count: calc(`${density} * 2`),
          seed,
          speed: [20, 52],
          gravity: [0, 20],
          drag: 0.2,
          lifetime: 4,
        },
      },
      { glow_dots: { radius: 2.2 } },
    ),
    trajectory(
      {
        converge: {
          from: points(
            `${density} * 2`,
            "76 + mod(t, 16) * 24",
            "44 + floor(t / 16) * 14",
          ),
          to: ring(`${density} * 2`, 94),
          duration: 4,
        },
      },
      dots(2.2),
    ),
    trajectory(
      {
        orbit: {
          center: [256, 152],
          radius: 96,
          count: calc(density),
          seed,
          angular_velocity: 0.9,
          lifetime: 4,
        },
      },
      { trails: { length: 0.2, width: 1.8 } },
    ),
    trajectory(
      {
        noise_drift: {
          origin: [180, 152],
          count: calc(density),
          seed,
          velocity: [28, 0],
          noise: [54, 0.7],
          lifetime: 4,
        },
      },
      dots(),
    ),
    simulation(
      [
        birth({ point: [156, 248] }, density, 30, 3, color, {
          velocity: [22, -100],
        }),
      ],
      [{ gravity: [0, 56] }, { wind: [16, 0] }, { drag: 0.12 }],
    ),
    group(
      circle([256, 152], 4, "#f6b863"),
      simulation(
        [cloud()],
        [{ attractor: { center: [256, 152], strength: 14000 } }, { drag: 1 }],
      ),
    ),
    null,
    simulation(
      [cloud()],
      [{ vortex: { center: [256, 152], strength: 28 } }, { drag: 0.2 }],
    ),
    simulation(
      [cloud()],
      [operator("seek_targets", { targets: ring(density, 88), strength: 2.4 })],
    ),
    simulation([cloud()], [curl(0.025, 14, 0.4), { drag: 0.3 }]),
    simulation(
      [cloud()],
      [
        operator("image_flow", {
          image: {
            size: [2, 2],
            rgba: [
              255, 160, 0, 255, 192, 255, 0, 255, 64, 128, 0, 255, 128, 32, 0,
              255,
            ],
          },
          bounds: [0, 0, 512, 304],
          strength: 26,
          phase_speed: 0.1,
        }),
        operator("curl_flow", { seed: curlSeed, frequency: 0.02, strength: 6 }),
        { drag: 0.3 },
      ],
    ),
    simulation(
      [cloud()],
      [
        operator("boids", {
          radius: 38,
          separation: 240,
          alignment: 1.8,
          cohesion: 0.6,
        }),
        { vortex: { center: [256, 152], strength: 14 } },
        { bounce_box: [40, 36, 472, 268] },
      ],
      { trails: { length: 0.22, width: 1.8 } },
    ),
    simulation(
      Array.from({ length: 24 }, (_, index) =>
        burst(
          1,
          [84 + index * 14, 152 + Math.sin(index * 0.55) * 28],
          12,
          4,
          [0.5, 0.75, 1, 1],
        ),
      ),
      [
        operator("spring", {
          rest_length: 14,
          stiffness: 16,
          damping: 1.8,
          closed: false,
        }),
        { drag: 0.15 },
      ],
      { plexus: { radius: 3, max_distance: 22 } },
    ),
    simulation(
      [
        birth(
          { disk: { center: [256, 152], radius: 72 } },
          `${density} * 0.4`,
          40,
          3.5,
          [0.9, 0.6, 0.3, 1],
          { size: 5 },
        ),
      ],
      [
        operator("particle_collisions", {
          restitution: 0.9,
          friction: 0,
          kill_on_hit: false,
        }),
        { bounce_box: [64, 48, 448, 256] },
      ],
      dots(1),
    ),
    group(
      rect([72, 52, 440, 252], "#243b31"),
      simulation(
        [burst(density, [256, 152], 100, 4, [0.3, 0.8, 1, 1])],
        [{ bounce_box: [72, 52, 440, 252] }],
      ),
    ),
    group(
      circle([256, 192], 60, "#426c59"),
      simulation(
        [
          birth(
            { rect: [232, 16, 280, 18] },
            density,
            0,
            3.8,
            [0.3, 0.8, 1, 1],
            { velocity: [0, 72], size: 2 },
          ),
        ],
        [
          { gravity: [0, 20] },
          operator("circle_collider", {
            center: [256, 192],
            radius: 60,
            response,
          }),
        ],
        dots(1),
      ),
    ),
    group(
      { path: { data: "M128 224 L256 136 L384 224 Z" }, fill: "#426c59" },
      simulation(
        [falling(density)],
        [
          { gravity: [0, 20] },
          operator("polygon_collider", {
            vertices: [
              [128, 224],
              [256, 136],
              [384, 224],
            ],
            response,
          }),
        ],
        dots(1),
      ),
    ),
    group(
      rect([179.2, 91.2, 332.8, 212.8], "#426c59"),
      simulation(
        [
          birth(
            { rect: [192, 16, 320, 18] },
            `${density} / 3`,
            0,
            3.8,
            [0.3, 0.8, 1, 1],
            { velocity: [0, 72], size: 2 },
          ),
        ],
        [
          { gravity: [0, 20] },
          operator("sdf_collider", { surface, resolution: [65, 39], response }),
        ],
        dots(1),
      ),
    ),
    group(
      rect([60, 244, 452, 247], "#426c59"),
      simulation(
        [falling(`${density} * 0.35`)],
        [
          { gravity: [0, 20] },
          operator("segment_collider", {
            start: [60, 244],
            end: [452, 244],
            response: { ...response, kill_on_hit: true },
          }),
        ],
        dots(1),
        {},
        {
          sub_emitters: [
            {
              on: "collision",
              emitter: burst(5, [0, 0], 38, 0.4, null, { velocity: [0, -30] }),
              inherit: "color",
              probability: 1,
              root_only: true,
            },
          ],
        },
      ),
    ),
    simulation(
      [
        burst(
          `floor((${density}) / 3) + 1`,
          [256, 152],
          22,
          2.6,
          [0.3, 0.8, 0.7, 1],
        ),
      ],
      [],
      dots(),
      {},
      {
        sub_emitters: ["birth", { age_reached: 1.3 }, "death"].map(
          (on, index) => ({
            on,
            emitter: burst(
              7,
              [0, 0],
              40,
              0.65,
              [
                [0.2, 0.6, 1, 1],
                [1, 0.65, 0.2, 1],
                [1, 0.25, 0.2, 1],
              ][index],
            ),
            inherit: "position",
            probability: 1,
            root_only: true,
          }),
        ),
      },
    ),
    group(
      ...[
        { polygon: { sides: 5 } },
        { ring: { thickness: 0.22 } },
        { capsule: { aspect: 2.4 } },
        { soft: { hardness: 0.4 } },
        { quad: { aspect: 1.5 } },
      ].map((shape, index) =>
        simulation(
          [
            birth(
              { disk: { center: [56 + index * 100, 152], radius: 28 } },
              `${density} / 8`,
              8,
              3,
              [0.3, 0.75, 0.95, 1],
            ),
          ],
          [],
          { shape: { radius: 5, shape } },
        ),
      ),
    ),
    ...[false, true].map((ribbon) =>
      simulation(
        [streaming()],
        [curl(0.02, 8, 0.3)],
        { trails: { length: 0.65, width: 3.2 } },
        {
          trails: {
            segments: 16,
            taper: curve(0, 1),
            fade: curve(0, 1),
            smooth: true,
            ribbon,
          },
        },
        { blend: "plus" },
      ),
    ),
    simulation(
      [
        birth(
          { rect: [112, 64, 400, 240] },
          `${density} * 0.5`,
          16,
          3.5,
          [0.25, 0.75, 0.85, 1],
        ),
      ],
      [],
      { plexus: { radius: 2, max_distance: 46 } },
    ),
    group(
      ...[null, 6].map((blades, index) =>
        simulation(
          [
            birth(
              { rect: [80 + index * 220, 64, 210 + index * 220, 240] },
              `${density} * 0.25`,
              8,
              3.8,
              [0.6, 0.45, 0.2, 0.75],
              { depth_range: [-2, 2] },
            ),
          ],
          [],
          {
            bokeh: {
              radius: 4,
              aperture: 4,
              focus: 0,
              blades,
              ring: 0.4,
              hardness: 0.6,
              highlight: 1.4,
            },
          },
          { parallax: [10, 0] },
          { blend: "screen" },
        ),
      ),
    ),
    null,
    simulation([spriteBirth()], [], sprites(34, { random: seed })),
    group(
      ...["over_lifetime", { fps: 2 }, { random: seed }, "custom"].flatMap(
        (mode, index) => [
          text(
            ["Lifetime", "FPS", "Random", "Custom"][index],
            13,
            42 + index * 124,
            228,
          ),
          simulation(
            [
              birth(
                { disk: { center: [68 + index * 124, 152], radius: 28 } },
                `${density} / 12`,
                5,
                3.8,
                null,
              ),
            ],
            phase(),
            sprites(32, mode),
          ),
        ],
      ),
    ),
    simulation(
      [spriteBirth()],
      [],
      sprites(
        54,
        { fps: 4 },
        Array.from({ length: 8 }, (_, index) => ({
          material: { smoke: { scale: 3, evolution: 1 } },
          size: 48,
          time: index * 0.2,
        })),
        false,
      ),
    ),
    group(
      simulation(
        [
          birth(
            { rect: [92, 24, 220, 26] },
            `${density} * 0.4`,
            4,
            3.2,
            [0.3, 0.7, 1, 1],
            { velocity: [0, 70] },
          ),
        ],
        [],
        { shape: { radius: 2, shape: { capsule: { aspect: 1 } } } },
        { stretch: [0.06, 8] },
      ),
      simulation(
        [
          birth(
            { rect: [292, 24, 420, 26] },
            `${density} * 0.4`,
            10,
            3.2,
            [1, 0.55, 0.2, 1],
            { velocity: [0, 30] },
          ),
        ],
        [{ gravity: [0, 18] }, ...phase()],
        { shape: { radius: 4, shape: { quad: { aspect: 1.6 } } } },
        { flip: true },
      ),
    ),
    simulation(
      [
        birth({ point: [80, 152] }, `${density} / 3`, 8, 3.5, null, {
          velocity: [85, 0],
        }),
      ],
      [
        { size_over_lifetime: { start: 0.5, end: 2.5 } },
        operator("color_curve", { colors, easing: "cubic_in_out" }),
        { intensity_over_lifetime: curve(1.5, 0.2) },
      ],
      { glow_dots: { radius: 2.4 } },
    ),
    group(
      ...["x", "y", "speed"].map((coordinate, index) =>
        simulation(
          [
            birth(
              { disk: { center: [68 + index * 124, 152], radius: 36 } },
              `${density} / 12`,
              18,
              3.5,
              null,
            ),
          ],
          [
            { gravity: [0, 8] },
            operator("color_field", {
              coordinate,
              range: [
                [24, 112],
                [100, 200],
                [0, 40],
              ][index],
              colors,
            }),
          ],
          dots(4),
        ),
      ),
      simulation(
        [
          birth(
            { disk: { center: [440, 152], radius: 36 } },
            `${density} / 12`,
            12,
            3.5,
            null,
          ),
        ],
        [{ temperature: curve(8500, 1200) }],
        { glow_dots: { radius: 3.2 } },
      ),
    ),
    simulation(
      [
        birth(
          { disk: { center: [256, 152], radius: 88 } },
          `${density} / 2`,
          12,
          3.8,
          null,
        ),
      ],
      [],
      {
        node_circles: {
          radius: 3,
          amplitude: 1,
          frequency: 4,
          fill: "#67dabb",
        },
      },
    ),
    simulation([timeBirth()], [], dots(), { prewarm: 1.5 }),
    simulation([timeBirth()], [], dots(), { loop_period: 1.8 }),
    simulation([timeBirth()], [], dots(), { time_remap: [0.35, 0] }),
    simulation([timeBirth()], [], dots(), {
      time_remap: [-0.75, 3],
      allow_reverse: true,
    }),
    group(
      text("1 worker", 14, 108, 268),
      text("4 workers", 14, 340, 268),
      ...[1, 4].map((workers, index) => ({
        ...simulation(
          [
            burst(
              `max(${density}, 64) * 2`,
              [256, 152],
              24,
              4,
              [0.4, 0.7, 1, 1],
            ),
          ],
          [{ gravity: [0, 8] }, { bounce_box: [180, 56, 332, 248] }],
          dots(1.8),
          { workers },
        ),
        translate: [index === 0 ? -116 : 116, 0],
      })),
    ),
  ];
  const radialEmitters = [],
    radialModifiers = [],
    radialGuides = [];
  [
    "constant",
    "linear",
    "inverse_square",
    {
      curve: {
        stops: [
          [0, 1],
          [0.5, 0.2],
          [1, 0],
        ],
        easing: "cubic_in_out",
      },
    },
  ].forEach((falloff, index) => {
    const center = [68 + index * 124, 152];
    radialEmitters.push(
      birth(
        { disk: { center, radius: 46 } },
        `${density} / 4`,
        0,
        3.8,
        [0.3, 0.7, 1, 1],
      ),
    );
    radialModifiers.push(
      operator("radial_force", {
        center,
        strength: index === 2 ? 9000 : 22,
        radius: 56,
        falloff,
      }),
    );
    radialGuides.push(
      circle(center, 56, "#243b31"),
      text(
        ["Constant", "Linear", "Inverse", "Curve"][index],
        13,
        center[0] - 26,
        224,
      ),
    );
  });
  studies[15] = group(
    ...radialGuides,
    simulation(radialEmitters, [...radialModifiers, { drag: 0.6 }]),
  );
  studies[34] = group(
    rect([20, 64, 492, 228], "#c9d9cf"),
    ...["source_over", "plus", "screen", "multiply"].flatMap((blend, index) => [
      text(
        ["Over", "Plus", "Screen", "Multiply"][index],
        13,
        44 + index * 124,
        248,
      ),
      simulation(
        [
          birth(
            { disk: { center: [68 + index * 124, 146], radius: 28 } },
            `${density} / 8`,
            4,
            3.8,
            [0.05, 0.2, 0.35, 0.5],
          ),
        ],
        [],
        dots(10),
        {},
        { blend },
      ),
    ]),
  );
  const result = document(
    "Particle Features",
    "PARTICLES",
    pages.map(([title, , ...lines]) => [title, ...lines]),
    studies,
  );
  result.params.count = 72;
  result.params.curl_seed = 20261027;
  result.params.segment_width = calc("(864 - 2 * ($pages - 1)) / $pages");
  result.tracks[0].items.forEach(({ scene }, page) => {
    const children = scene.content.children;
    children
      .find((node) => node.name === "Page captions")
      .children[0].children.push({
        text: {
          content: pages[page][1],
          family: "",
          size: 11,
          weight: 600,
          letter_spacing: 1.2,
        },
        fill: "#56635c",
        translate: [48, 288],
      });
    children.find((node) => node.name === "Footer").children = pages.flatMap(
      (_, index) => {
        const left = `48 + ${index} * ($segment_width + 2)`;
        const top = `494 - if(@clock.translate.y == ${index}, 2, 1)`;
        const bottom = `494 + if(@clock.translate.y == ${index}, 2, 1)`;
        return [
          rect(
            expr([left, top, `${left} + $segment_width`, bottom]),
            "#c3cbc5",
          ),
          rect(
            expr([
              left,
              top,
              `${left} + $segment_width * clamp(@clock.translate.x * $pages - ${index}, 0, 1)`,
              bottom,
            ]),
            "#d64b36",
          ),
        ];
      },
    );
  });
  return result;
};
