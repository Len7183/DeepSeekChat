// 生成 DeepSeekChat 全尺寸图标（FR-17）：
// 素材：DeepSeek 官网 favicon（鲸鱼 logo，商标归 DeepSeek 官方所有，仅用于标识归属）。
// 流程：解析官方 favicon.ico 的 BMP 条目 → sharp 解码 → 裁切居中 → 圆角蒙版 →
//       输出 256/128/64/48/32/16 PNG → 直接封装成 icons/icon.ico（PNG 压缩条目）。
import { writeFileSync, mkdirSync, existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "icons");
const sourceCandidates = [
  join(root, "scripts", "ds-favicon.ico"),
  // 注意：icon.ico（脚本自身产物）只作为无原始素材时的应急来源，且会在
  // 产物上叠加圆角/重采样造成代际损失——原始素材缺失时给出醒目提示
  join(root, "src-tauri", "icons", "icon.ico"),
];
const source = sourceCandidates.find((p) => existsSync(p));
if (!source) {
  console.error("未找到素材文件，请先下载 DeepSeek 官网 favicon 到 scripts/ds-favicon.ico");
  process.exit(1);
}
if (source === sourceCandidates[1]) {
  console.warn("警告：原始素材缺失，正在基于自身产物再生成（存在代际损失，建议恢复 scripts/ds-favicon.ico）");
}
console.log("素材:", source);

// ---- 1. 解析 ICO 容器，提取最佳条目并补上文件头使其成为合法 BMP ----
// 边界防御：容器结构完全不可信（截断/越界偏移），所有读取前先校验长度，
// Buffer.subarray 会静默钳制越界下标，不检查就会把垃圾数据当合法条目
const ico = readFileSync(source);
if (ico.length < 6 || ico.readUInt16LE(0) !== 0 || ico.readUInt16LE(2) !== 1) {
  console.error("不是合法的 ICO 容器（头缺失或类型不是图标）");
  process.exit(1);
}
const entries = ico.readUInt16LE(4);
let best = null;
for (let i = 0; i < entries; i++) {
  const off = 6 + i * 16;
  if (off + 16 > ico.length) break; // 目录截断
  const size = ico.readUInt32LE(off + 8);
  const dataOff = ico.readUInt32LE(off + 12);
  if (dataOff + size > ico.length) continue; // 数据越界，跳过该条目
  const data = ico.subarray(dataOff, dataOff + size);
  // 以目录项声明的分辨率挑最佳（0 = 256）：字节数只是分辨率代理，
  // BMP 条目自带掩码/填充开销，按字节挑可能选中更低分辨率的 BMP
  const wDecl = ico.readUInt8(off) || 256;
  const hDecl = ico.readUInt8(off + 1) || 256;
  const better =
    !best || wDecl * hDecl > best.w * best.h || (wDecl * hDecl === best.w * best.h && size > best.size);
  if (better) best = { size, data, w: wDecl, h: hDecl };
}
if (!best) {
  console.error("ICO 无有效条目（全部越界或容器为空）");
  process.exit(1);
}
console.log("素材:", source, `(${best.w}x${best.h})`);

// BITMAPINFOHEADER 起始特征（biSize=40 且 biPlanes=1）
const isBmp = best.data.length > 46 && best.data.readUInt32LE(0) === 40 && best.data.readUInt16LE(12) === 1;
// 完整 8 字节 PNG 签名（只查前两字节会把碰巧 0x89 0x50 开头的杂项当 PNG）
const PNG_SIG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const isPng = best.data.subarray(0, 8).equals(PNG_SIG);

let sourceBuffer;
let rawInput = null;
if (isPng) {
  sourceBuffer = best.data;
} else if (isBmp) {
  // 手动解码 32bpp BI_RGB 的 ICO-BMP（部分 sharp 构建无 BMP 加载器）
  const width = best.data.readInt32LE(4);
  const heightRaw = best.data.readInt32LE(8); // ICO BMP 为 2 倍高（含 AND 掩码）
  const bpp = best.data.readUInt16LE(14);
  const compression = best.data.readUInt32LE(16);
  if (bpp !== 32 || compression !== 0) {
    console.error(`不支持的 BMP 格式: bpp=${bpp} compression=${compression}`);
    process.exit(1);
  }
  if (width <= 0 || heightRaw <= 0) {
    // 负 biHeight（top-down）/负宽度：Buffer.alloc 收到负数抛裸 RangeError
    console.error(`异常 BMP 尺寸: width=${width} biHeight=${heightRaw}`);
    process.exit(1);
  }
  const height = Math.floor(heightRaw / 2);
  // 尺寸与数据长度交叉校验：头声明的像素量超出条目数据即为损坏
  const px = 40; // BITMAPINFOHEADER 之后
  if (px + heightRaw * width * 4 > best.data.length) {
    console.error(`BMP 数据不足: 声明 ${width}x${height}，实际 ${best.data.length} 字节`);
    process.exit(1);
  }
  const stride = width * 4; // 32bpp 行宽已 4 字节对齐
  const rgba = Buffer.alloc(width * height * 4);
  for (let y = 0; y < height; y++) {
    // ICO BMP 自下而上存储
    const src = px + (height - 1 - y) * stride;
    for (let x = 0; x < width; x++) {
      const s = src + x * 4;
      const d = (y * width + x) * 4;
      rgba[d] = best.data[s + 2]; // R
      rgba[d + 1] = best.data[s + 1]; // G
      rgba[d + 2] = best.data[s]; // B
      rgba[d + 3] = best.data[s + 3]; // A
    }
  }
  sourceBuffer = rgba;
  rawInput = { width, height, channels: 4 };
} else {
  console.error("无法识别的 ICO 条目格式");
  process.exit(1);
}

// ---- 2. 解码为 RGBA，缩放到目标尺寸，应用圆角蒙版 ----
// 损坏素材/缺失加载器在此统一走友好报错，而不是未处理 Promise 拒绝裸栈
let meta;
try {
  const base0 = rawInput ? sharp(sourceBuffer, { raw: rawInput }) : sharp(sourceBuffer, { density: 300 });
  meta = await base0.metadata();
} catch (e) {
  console.error(`素材解码失败（文件损坏或 sharp 缺少对应加载器）: ${e.message}`);
  process.exit(1);
}
console.log(`素材解码: ${meta.width}x${meta.height}`);
const base = (rawInput ? sharp(sourceBuffer, { raw: rawInput }) : sharp(sourceBuffer, { density: 300 })).ensureAlpha();

const SIZES = [256, 128, 64, 48, 32, 16];
const radiusPct = 0.22; // 圆角比例

mkdirSync(outDir, { recursive: true });
// 各尺寸管道相互独立，并行处理缩短耗时（map 保序，pngs 与 SIZES 对齐）
const pngs = await Promise.all(
  SIZES.map(async (size) => {
    const radius = Math.round(size * radiusPct);
    const mask = Buffer.from(
      `<svg width="${size}" height="${size}"><rect x="0" y="0" width="${size}" height="${size}" rx="${radius}" ry="${radius}"/></svg>`
    );
    try {
      const png = await base
        .clone()
        .resize(size, size, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
        .composite([{ input: mask, blend: "dest-in" }])
        .png()
        .toBuffer();
      writeFileSync(join(outDir, `icon-${size}.png`), png);
      return png;
    } catch (e) {
      console.error(`生成 ${size}px 图标失败（sharp 缺少 SVG 加载器？）: ${e.message}`);
      process.exit(1);
    }
  })
);

// ---- 3. 合成 icon.ico：直接封装上一步的 PNG（PNG 压缩条目，Win10/11、资源管理器、
//         托盘与 NSIS 向导均支持）。自己写 6 字节头 + 每图 16 字节目录项即可，
//         不必再引一个依赖把 PNG 解码成 BMP 条目（那会让 31KB 膨胀到 370KB）。 ----
function buildIco(buffers, sizes) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // 保留位
  header.writeUInt16LE(1, 2); // 1 = 图标
  header.writeUInt16LE(buffers.length, 4);

  const dir = Buffer.alloc(16 * buffers.length);
  let offset = header.length + dir.length;
  buffers.forEach((png, i) => {
    const size = sizes[i];
    const at = i * 16;
    const dim = size >= 256 ? 0 : size; // 256 在 ICO 目录项里记作 0
    dir.writeUInt8(dim, at); // 宽
    dir.writeUInt8(dim, at + 1); // 高
    dir.writeUInt8(0, at + 2); // 调色板数
    dir.writeUInt8(0, at + 3); // 保留
    dir.writeUInt16LE(1, at + 4); // 颜色平面
    dir.writeUInt16LE(32, at + 6); // 位深
    dir.writeUInt32LE(png.length, at + 8); // 数据长度
    dir.writeUInt32LE(offset, at + 12); // 数据偏移
    offset += png.length;
  });

  return Buffer.concat([header, dir, ...buffers]);
}

const icoOut = buildIco(pngs, SIZES);
writeFileSync(join(outDir, "icon.ico"), icoOut);
console.log(
  `已生成: ${join(outDir, "icon.ico")}（${icoOut.length} 字节 / ${SIZES.join("/")} PNG 条目）+ ${SIZES.join("/")}.png`
);
