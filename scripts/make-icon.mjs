// Rebuilds assets/brand/miru.ico from the icon SVGs (miru-icon-small.svg for 32 px and below,
// miru-icon.svg above). Needs the resvg renderer, installed outside the repo:
//
//   mkdir $env:TEMP\miru-icon; cd $env:TEMP\miru-icon; bun init -y; bun add @resvg/resvg-js
//   node scripts/make-icon.mjs $env:TEMP\miru-icon assets/brand assets/brand/miru.ico $env:TEMP\miru-icon\png
//
import { createRequire } from "node:module";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const [depsDir, brandDir, outIco, pngDir] = process.argv.slice(2);
const require = createRequire(join(depsDir, "package.json"));
const { Resvg } = require("@resvg/resvg-js");

const regular = readFileSync(join(brandDir, "miru-icon.svg"), "utf8");
const small = readFileSync(join(brandDir, "miru-icon-small.svg"), "utf8");
const sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
mkdirSync(pngDir, { recursive: true });

const images = sizes.map((size) => {
  const svg = size <= 32 ? small : regular;
  const png = new Resvg(svg, { fitTo: { mode: "width", value: size } }).render().asPng();
  writeFileSync(join(pngDir, `miru-${size}.png`), png);
  return { size, png };
});

// ICONDIR (6 bytes) + ICONDIRENTRY (16 bytes each) + PNG data.
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0); // reserved
header.writeUInt16LE(1, 2); // type: icon
header.writeUInt16LE(images.length, 4);
let offset = 6 + 16 * images.length;
const entries = images.map(({ size, png }) => {
  const e = Buffer.alloc(16);
  e.writeUInt8(size >= 256 ? 0 : size, 0); // width (0 means 256)
  e.writeUInt8(size >= 256 ? 0 : size, 1); // height
  e.writeUInt8(0, 2); // palette colours
  e.writeUInt8(0, 3); // reserved
  e.writeUInt16LE(1, 4); // colour planes
  e.writeUInt16LE(32, 6); // bits per pixel
  e.writeUInt32LE(png.length, 8);
  e.writeUInt32LE(offset, 12);
  offset += png.length;
  return e;
});
writeFileSync(outIco, Buffer.concat([header, ...entries, ...images.map((i) => i.png)]));
console.log(`wrote ${outIco} with ${images.length} sizes (${offset} bytes)`);
