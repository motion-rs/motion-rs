#!/usr/bin/env node
"use strict";
const fs = require("node:fs");
const [style = "teal-orange", destination, edgeText = "33"] =
  process.argv.slice(2);
const edge = Number(edgeText);
if (
  !destination ||
  !Number.isInteger(edge) ||
  edge < 2 ||
  edge > 64 ||
  !["teal-orange", "bleach", "night", "identity"].includes(style)
) {
  throw new Error(
    "Usage: node tools/generate-lut.cjs teal-orange|bleach|night|identity output.cube [2..64]",
  );
}
function grade(rgb) {
  const luma = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  if (style === "identity") return rgb;
  if (style === "bleach")
    return rgb.map(
      (value) => (luma + (value - luma) * 0.35 - 0.18) * 1.15 + 0.18,
    );
  if (style === "night") return [rgb[0] * 0.82, rgb[1] * 0.94, rgb[2] * 1.08];
  const blend = Math.max(0, Math.min(1, luma));
  const shadow = [-0.018, 0.012, 0.025];
  const highlight = [0.025, 0.008, -0.012];
  return rgb.map(
    (value, channel) =>
      value + shadow[channel] * (1 - blend) + highlight[channel] * blend,
  );
}
const lines = [
  `TITLE "Motion original ${style}"`,
  `LUT_3D_SIZE ${edge}`,
  "DOMAIN_MIN 0 0 0",
  "DOMAIN_MAX 1 1 1",
];
for (let blue = 0; blue < edge; blue++)
  for (let green = 0; green < edge; green++)
    for (let red = 0; red < edge; red++) {
      lines.push(
        grade([red, green, blue].map((value) => value / (edge - 1)))
          .map((value) => value.toFixed(9))
          .join(" "),
      );
    }
fs.writeFileSync(destination, `${lines.join("\n")}\n`, { flag: "wx" });
