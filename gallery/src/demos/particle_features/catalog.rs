//! Stable feature-page ordering, descriptions and corresponding specimen builders.

use super::{Page, appearance, burst, disk, emission, physics, point, rectangle, timing};

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Point emitter",
        category: "EMISSION / POINT",
        lines: [
            "Particles share an origin",
            "but receive independently",
            "seeded launch directions.",
        ],
        build: point,
    },
    Page {
        title: "Disk emitter",
        category: "EMISSION / AREA",
        lines: [
            "Births cover the disk area",
            "uniformly, rather than",
            "just its circumference.",
        ],
        build: disk,
    },
    Page {
        title: "Rect emitter",
        category: "EMISSION / AREA",
        lines: [
            "A rectangular source",
            "distributes births across",
            "its full local surface.",
        ],
        build: rectangle,
    },
    Page {
        title: "Timed burst",
        category: "EMISSION / BURST",
        lines: [
            "One bounded population",
            "launches at an explicit",
            "time on the fixed grid.",
        ],
        build: burst,
    },
    Page {
        title: "Path emitter",
        category: "EMISSION / PATH",
        lines: [
            "An animated arc-length",
            "source launches particles",
            "along the curve tangent.",
        ],
        build: emission::path,
    },
    Page {
        title: "Vector surface",
        category: "EMISSION / COLOR",
        lines: [
            "Area-uniform births keep",
            "the vector surface colors",
            "without altering density.",
        ],
        build: emission::surface,
    },
    Page {
        title: "Image alpha",
        category: "EMISSION / IMAGE",
        lines: [
            "Image alpha controls",
            "where particles are born;",
            "pixel colors are inherited.",
        ],
        build: emission::image_alpha,
    },
    Page {
        title: "Image luminance",
        category: "EMISSION / IMAGE",
        lines: [
            "Bright source pixels emit",
            "more particles than dark",
            "pixels in the same image.",
        ],
        build: emission::image_luminance,
    },
    Page {
        title: "Text surface",
        category: "EMISSION / GLYPHS",
        lines: [
            "Shaped glyph outlines",
            "become filled emission",
            "surfaces, not bitmaps.",
        ],
        build: emission::text_study,
    },
    Page {
        title: "Ballistic",
        category: "ANALYTIC / GRAVITY",
        lines: [
            "Gravity and linear drag",
            "are evaluated together",
            "at any requested time.",
        ],
        build: emission::ballistic,
    },
    Page {
        title: "Converge",
        category: "ANALYTIC / TARGETS",
        lines: [
            "A complete point set",
            "eases from a grid into",
            "a circular target shape.",
        ],
        build: emission::converge,
    },
    Page {
        title: "Orbit",
        category: "ANALYTIC / PHASE",
        lines: [
            "Seeded orbital phases",
            "provide exact positions",
            "and tangent velocities.",
        ],
        build: emission::orbit,
    },
    Page {
        title: "Noise drift",
        category: "ANALYTIC / NOISE",
        lines: [
            "Smooth noise displaces",
            "each stable particle ID",
            "without stepping history.",
        ],
        build: emission::noise,
    },
    Page {
        title: "Gravity / wind",
        category: "FORCES / DRAG",
        lines: [
            "Gravity bends the flight;",
            "wind pushes sideways",
            "while drag damps speed.",
        ],
        build: physics::gravity,
    },
    Page {
        title: "Attractor",
        category: "FORCES / DISTANCE",
        lines: [
            "A softened inverse-square",
            "field pulls particles",
            "toward a shared center.",
        ],
        build: physics::attractor,
    },
    Page {
        title: "Radial falloff",
        category: "FORCES / FALLOFF",
        lines: [
            "Constant, linear, inverse",
            "square and custom curves",
            "control attraction range.",
        ],
        build: physics::radial,
    },
    Page {
        title: "Vortex",
        category: "FORCES / TANGENT",
        lines: [
            "Tangential acceleration",
            "turns a seeded cloud",
            "around a softened center.",
        ],
        build: physics::vortex,
    },
    Page {
        title: "Seek targets",
        category: "FORCES / TARGETS",
        lines: [
            "Stable particle IDs seek",
            "matching targets using",
            "velocity correction.",
        ],
        build: physics::seek,
    },
    Page {
        title: "Curl noise",
        category: "FORCES / PERLIN",
        lines: [
            "An evolving curl field",
            "adds divergence-free",
            "seeded accelerations.",
        ],
        build: physics::curl,
    },
    Page {
        title: "Flow fields",
        category: "FORCES / IMAGE",
        lines: [
            "An image vector map adds",
            "flow to particles while",
            "a curl field evolves.",
        ],
        build: physics::flow,
    },
    Page {
        title: "Boids",
        category: "GROUP / FLOCKING",
        lines: [
            "Neighbors separate, align",
            "and cohere using a stable",
            "population snapshot.",
        ],
        build: physics::boids,
    },
    Page {
        title: "Spring chain",
        category: "GROUP / HOOKE",
        lines: [
            "Consecutive stable IDs",
            "are coupled by springs",
            "with explicit damping.",
        ],
        build: physics::spring,
    },
    Page {
        title: "Particle contacts",
        category: "GROUP / COLLISION",
        lines: [
            "Equal-mass particles",
            "exchange impulses and",
            "separate on contact.",
        ],
        build: physics::contacts,
    },
    Page {
        title: "Bounce box",
        category: "CONTACT / BOUNDS",
        lines: [
            "Predicted positions",
            "reflect inside a bounded",
            "local rectangle.",
        ],
        build: physics::bounce,
    },
    Page {
        title: "Circle collider",
        category: "CONTACT / SWEPT",
        lines: [
            "Falling particles strike",
            "a solid circular obstacle",
            "with finite contact size.",
        ],
        build: physics::circle,
    },
    Page {
        title: "Polygon collider",
        category: "CONTACT / SWEPT",
        lines: [
            "A prepared polygon",
            "deflects incoming discs",
            "along its edge normals.",
        ],
        build: physics::polygon,
    },
    Page {
        title: "SDF collider",
        category: "CONTACT / GRID",
        lines: [
            "A bounded distance grid",
            "and exact edge guards",
            "resolve surface contacts.",
        ],
        build: physics::sdf,
    },
    Page {
        title: "Collision events",
        category: "EVENT / SUB-EMITTER",
        lines: [
            "Floor contacts trigger",
            "small inherited bursts",
            "without recursive rules.",
        ],
        build: physics::events,
    },
    Page {
        title: "Lifecycle events",
        category: "EVENT / STAGES",
        lines: [
            "Birth, age and death",
            "trigger distinct colored",
            "bursts from root particles.",
        ],
        build: physics::lifecycle,
    },
    Page {
        title: "Shape gallery",
        category: "STYLE / GEOMETRY",
        lines: [
            "Polygons, rings, capsules,",
            "soft discs and quads need",
            "no image textures.",
        ],
        build: appearance::shapes,
    },
    Page {
        title: "Historical trails",
        category: "STYLE / HISTORY",
        lines: [
            "Shared historical samples",
            "form tapered, faded and",
            "smoothed trail segments.",
        ],
        build: appearance::trails,
    },
    Page {
        title: "Ribbon trails",
        category: "STYLE / RIBBON",
        lines: [
            "Continuous flat-ended",
            "strips join smoothed",
            "historical positions.",
        ],
        build: appearance::ribbons,
    },
    Page {
        title: "Plexus",
        category: "STYLE / NEIGHBORS",
        lines: [
            "Stable nearby pairs",
            "form a moving network",
            "with bounded link work.",
        ],
        build: appearance::plexus,
    },
    Page {
        title: "Bokeh / depth",
        category: "STYLE / APERTURE",
        lines: [
            "Depth changes defocus",
            "and parallax with stable",
            "far-to-near sorting.",
        ],
        build: appearance::bokeh,
    },
    Page {
        title: "Blend modes",
        category: "STYLE / COMPOSITE",
        lines: [
            "SourceOver, Plus, Screen",
            "and Multiply combine",
            "the same colored dots.",
        ],
        build: appearance::blends,
    },
    Page {
        title: "Sprite atlas",
        category: "STYLE / MATERIALS",
        lines: [
            "Five generated materials",
            "share a padded atlas",
            "with seeded selection.",
        ],
        build: appearance::atlas,
    },
    Page {
        title: "Sprite sheets",
        category: "STYLE / FRAME SELECT",
        lines: [
            "Lifetime, FPS, random",
            "and custom state select",
            "frames without a cursor.",
        ],
        build: appearance::sheets,
    },
    Page {
        title: "Evolving smoke",
        category: "STYLE / FLIPBOOK",
        lines: [
            "Explicit-time smoke",
            "materials form a sampled",
            "animated sprite sheet.",
        ],
        build: appearance::smoke,
    },
    Page {
        title: "Stretch / flip",
        category: "STYLE / VELOCITY",
        lines: [
            "Speed stretches capsules;",
            "age and seeded phases",
            "flip falling quads.",
        ],
        build: appearance::stretch,
    },
    Page {
        title: "Lifetime curves",
        category: "APPEARANCE / AGE",
        lines: [
            "Size, eased color and",
            "HDR intensity follow",
            "normalized particle age.",
        ],
        build: appearance::lifetime,
    },
    Page {
        title: "Color / heat",
        category: "APPEARANCE / FIELD",
        lines: [
            "Position and speed sample",
            "color fields; temperature",
            "maps Kelvin to linear RGB.",
        ],
        build: appearance::color,
    },
    Page {
        title: "Custom style",
        category: "STYLE / CALLBACK",
        lines: [
            "A Rust callback converts",
            "particle state into",
            "ordinary animated nodes.",
        ],
        build: appearance::custom,
    },
    Page {
        title: "Prewarm",
        category: "TIME / LEAD-IN",
        lines: [
            "A prepared lead-in fills",
            "the source before the",
            "visible clip begins.",
        ],
        build: timing::prewarm,
    },
    Page {
        title: "Loop period",
        category: "TIME / TWO PHASES",
        lines: [
            "Offset populations use",
            "crossfades to hide the",
            "wrap of a finite source.",
        ],
        build: timing::looped,
    },
    Page {
        title: "Time remap",
        category: "TIME / SLOW MOTION",
        lines: [
            "An explicit query scale",
            "slows the source without",
            "changing its fixed step.",
        ],
        build: timing::slow,
    },
    Page {
        title: "Reverse queries",
        category: "TIME / REPLAY",
        lines: [
            "Negative time mapping",
            "revives historical states",
            "with explicit opt-in.",
        ],
        build: timing::reverse,
    },
    Page {
        title: "Workers / replay",
        category: "EXECUTION / PARITY",
        lines: [
            "One and four workers",
            "produce matching states",
            "shown side by side.",
        ],
        build: timing::workers,
    },
];
