const fs = require("fs");
const path = require("path");

const outDir = path.join(__dirname, "..", "src-tauri", "icons");
fs.mkdirSync(outDir, { recursive: true });

const size = 32;
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(1, 4);

const dibSize = 40;
const bpp = 32;
const rowBytes = Math.floor((size * bpp + 31) / 32) * 4;
const imageSize = dibSize + rowBytes * size;

const entry = Buffer.alloc(16);
entry[0] = size;
entry[1] = size;
entry[2] = 0;
entry[3] = 0;
entry.writeUInt16LE(1, 4);
entry.writeUInt16LE(bpp, 6);
entry.writeUInt32LE(imageSize, 8);
entry.writeUInt32LE(22, 12);

const dib = Buffer.alloc(dibSize);
dib.writeUInt32LE(40, 0);
dib.writeInt32LE(size, 4);
dib.writeInt32LE(size * 2, 8);
dib.writeUInt16LE(1, 12);
dib.writeUInt16LE(bpp, 14);

const pixels = Buffer.alloc(rowBytes * size);
pixels.fill(0);

const ico = Buffer.concat([header, entry, dib, pixels]);
fs.writeFileSync(path.join(outDir, "icon.ico"), ico);

const pngHeader = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
fs.writeFileSync(path.join(outDir, "32x32.png"), pngHeader);
fs.writeFileSync(path.join(outDir, "128x128.png"), pngHeader);

console.log("Created src-tauri/icons/icon.ico and placeholders");
