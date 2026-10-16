// Deterministic isometric block fields: the hero ore vein and generated server banners.

export interface Face {
  points: string;
  shade: 'top' | 'left' | 'right';
}

export interface Cube {
  faces: Face[];
  /** Position in the vein's left-to-right fill order, or -1 for host rock. */
  ore: number;
  /** Small per-cube lightness offset so rock reads as stone rather than plastic. */
  grain: number;
  /** 0 at the back of the field to 1 at the front, for depth shading. */
  depth: number;
  /** Second material in banners; unused by the ore field. */
  accent: boolean;
}

export interface Field {
  cubes: Cube[];
  oreCount: number;
  width: number;
  height: number;
}

/** Seeded PRNG (mulberry32) so a name always yields the same art. */
function random(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function hashString(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return h >>> 0;
}

interface Shape {
  w: number;
  d: number;
  size: number;
  heights: number[][];
  isOre: (x: number, y: number, z: number) => boolean;
  isAccent: (x: number, y: number, z: number) => boolean;
  grain: () => number;
}

function project(size: number, ox: number, oy: number) {
  const cx = size * Math.cos(Math.PI / 6);
  return (x: number, y: number, z: number) => `${(ox + (x - y) * cx).toFixed(1)},${(oy + (x + y) * size * 0.5 - z * size).toFixed(1)}`;
}

function build(shape: Shape): Field {
  const { w, d, size, heights } = shape;
  const top = Math.max(...heights.flat());
  const cx = size * Math.cos(Math.PI / 6);
  const width = (w + d) * cx;
  const height = (w + d) * size * 0.5 + top * size;
  const p = project(size, d * cx, top * size);
  const h = (x: number, y: number) => (x < w && y < d ? heights[y]![x]! : 0);

  const cubes: (Cube & { order: number; sx: number })[] = [];
  for (let s = 0; s < w + d - 1; s++) {
    for (let x = 0; x < w; x++) {
      const y = s - x;
      if (y < 0 || y >= d) continue;
      for (let z = 0; z < h(x, y); z++) {
        const faces: Face[] = [];
        if (z === h(x, y) - 1) faces.push({ shade: 'top', points: [p(x, y, z + 1), p(x + 1, y, z + 1), p(x + 1, y + 1, z + 1), p(x, y + 1, z + 1)].join(' ') });
        if (h(x, y + 1) <= z) faces.push({ shade: 'left', points: [p(x, y + 1, z + 1), p(x + 1, y + 1, z + 1), p(x + 1, y + 1, z), p(x, y + 1, z)].join(' ') });
        if (h(x + 1, y) <= z) faces.push({ shade: 'right', points: [p(x + 1, y, z + 1), p(x + 1, y + 1, z + 1), p(x + 1, y + 1, z), p(x + 1, y, z)].join(' ') });
        if (faces.length === 0) continue;
        const ore = shape.isOre(x, y, z);
        cubes.push({ faces, ore: ore ? 0 : -1, grain: shape.grain(), depth: s / (w + d - 2), accent: !ore && shape.isAccent(x, y, z), order: cubes.length, sx: x - y });
      }
    }
  }
  const ores = cubes.filter((c) => c.ore >= 0).sort((a, b) => a.sx - b.sx || a.order - b.order);
  ores.forEach((c, i) => (c.ore = i));
  return { cubes, oreCount: ores.length, width, height };
}

function heightmap(w: number, d: number, rand: () => number, hills: number, peak: number, relief = 1.9): number[][] {
  const bumps = Array.from({ length: hills }, () => ({ x: rand() * w, y: rand() * d, r: 2 + rand() * 3.5, a: 0.6 + rand() }));
  return Array.from({ length: d }, (_, y) =>
    Array.from({ length: w }, (_, x) => {
      let v = 0.8;
      for (const b of bumps) v += b.a * Math.exp(-((x - b.x) ** 2 + (y - b.y) ** 2) / (b.r * b.r));
      return Math.max(1, Math.min(peak, Math.round(v * relief)));
    }),
  );
}

/** The launchpad's outcrop: host rock crossed by one cinnabar vein. */
export function oreField(size: number, seed = 2026, relief = 1.7): Field {
  const rand = random(seed);
  const w = 11;
  const d = 9;
  const heights = heightmap(w, d, rand, 6, 7, relief);
  return build({
    w,
    d,
    size,
    heights,
    isOre: (x, y, z) => Math.abs(x * 0.55 - y * 0.8 + z * 0.9 - 2.2 + Math.sin(x * 0.9) * 0.9) < 0.75,
    isAccent: () => false,
    grain: () => (rand() - 0.5) * 0.06,
  });
}

/** A small generated island for a server without artwork, tinted from its name. */
export function bannerField(name: string, size: number): Field {
  const rand = random(hashString(name));
  const w = 8;
  const d = 8;
  return build({
    w,
    d,
    size,
    heights: heightmap(w, d, rand, 4, 5),
    isOre: () => false,
    isAccent: () => rand() < 0.08,
    grain: () => (rand() - 0.5) * 0.08,
  });
}

/** Three face tones for a hue, lit from the top as the brand mark is. */
export function hueShades(hue: number) {
  return {
    ground: `hsl(${hue} 26% 13%)`,
    top: `hsl(${hue} 38% 50%)`,
    right: `hsl(${hue} 38% 38%)`,
    left: `hsl(${hue} 40% 25%)`,
    accentTop: `hsl(${(hue + 150) % 360} 55% 62%)`,
    accentRight: `hsl(${(hue + 150) % 360} 50% 48%)`,
    accentLeft: `hsl(${(hue + 150) % 360} 50% 32%)`,
  };
}

/** Renders a banner field to standalone SVG markup at a 2:1 frame. */
export function bannerSvg(name: string, hue: number): string {
  const field = bannerField(name, 30);
  const c = hueShades(hue);
  const vw = 640;
  const vh = 320;
  const scale = Math.min((vw * 0.78) / field.width, (vh * 0.92) / field.height);
  const tx = (vw - field.width * scale) / 2;
  const ty = (vh - field.height * scale) / 2 + 10;
  const fill = (cube: Cube, shade: Face['shade']) => {
    if (cube.accent) return shade === 'top' ? c.accentTop : shade === 'right' ? c.accentRight : c.accentLeft;
    return c[shade];
  };
  const polys = field.cubes
    .flatMap((cube) => cube.faces.map((f) => `<polygon points="${f.points}" fill="${fill(cube, f.shade)}"/>`))
    .join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${vw} ${vh}" preserveAspectRatio="xMidYMid slice"><rect width="${vw}" height="${vh}" fill="${c.ground}"/><g transform="translate(${tx.toFixed(1)} ${ty.toFixed(1)}) scale(${scale.toFixed(3)})" stroke="rgba(0,0,0,.18)" stroke-width="0.6" stroke-linejoin="round">${polys}</g></svg>`;
}

export function isoBannerDataUri(name: string, hue: number): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(bannerSvg(name, hue))}`;
}
