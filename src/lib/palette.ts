// The pastel palette, and how an app's manifest
// color becomes a tile.

export interface Swatch {
  bg: string;
  strong: string;
  ink: string;
}

export const COLOR_KEYS = ["mint", "peach", "lavender", "sky", "butter", "rose", "sage", "lilac", "apricot", "mist"] as const;
export type ColorKey = (typeof COLOR_KEYS)[number];

const LIGHT: Record<ColorKey, Swatch> = {
  mint: { bg: "#E2F5EB", strong: "#46B086", ink: "#1D5741" },
  peach: { bg: "#FDE7DC", strong: "#E8916A", ink: "#773C20" },
  lavender: { bg: "#ECE7FB", strong: "#8D79DF", ink: "#42338B" },
  sky: { bg: "#E1EEFC", strong: "#5797DE", ink: "#1D4B7C" },
  butter: { bg: "#FAF1D2", strong: "#D2A82E", ink: "#6A500D" },
  rose: { bg: "#FCE3E9", strong: "#DF7592", ink: "#7A253E" },
  sage: { bg: "#E7EEDF", strong: "#86A271", ink: "#3B4F2D" },
  lilac: { bg: "#F3E5F7", strong: "#B679C9", ink: "#5B2B6D" },
  apricot: { bg: "#FDEAD5", strong: "#E99F55", ink: "#764311" },
  mist: { bg: "#E7EBF0", strong: "#8897AB", ink: "#374252" },
};

const DARK: Record<ColorKey, Swatch> = {
  mint: { bg: "#1E3A2F", strong: "#7AD6AF", ink: "#CDF2E1" },
  peach: { bg: "#41291E", strong: "#F2A887", ink: "#FBDACB" },
  lavender: { bg: "#2E2848", strong: "#B2A2F5", ink: "#E2DBFD" },
  sky: { bg: "#1E2F47", strong: "#89BCF2", ink: "#D4E7FB" },
  butter: { bg: "#3A331E", strong: "#EACA68", ink: "#F6EAC2" },
  rose: { bg: "#42232C", strong: "#F29AB1", ink: "#FBD8E1" },
  sage: { bg: "#293224", strong: "#A8C394", ink: "#DCE7D3" },
  lilac: { bg: "#38273E", strong: "#D19EE1", ink: "#F0DBF6" },
  apricot: { bg: "#3F2F1D", strong: "#F3BA7F", ink: "#FBE2C8" },
  mist: { bg: "#292E36", strong: "#A8B4C4", ink: "#DDE2E9" },
};

export function swatch(key: string, dark: boolean): Swatch {
  const k = (COLOR_KEYS as readonly string[]).includes(key) ? (key as ColorKey) : "mist";
  return (dark ? DARK : LIGHT)[k];
}

// ---- manifest colors --------------------------------------------------------------

/** A manifest color as hex: hex stays, a palette name becomes its pastel. */
export function colorHex(color: string): string {
  if ((COLOR_KEYS as readonly string[]).includes(color)) return DARK[color as ColorKey].strong;
  if (/^#[0-9a-f]{3}$/i.test(color)) return "#" + [...color.slice(1)].map((c) => c + c).join("");
  if (/^#[0-9a-f]{6}$/i.test(color)) return color;
  return DARK.mist.strong;
}

function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function toHsl([r, g, b]: [number, number, number]): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, s, l];
}

function luminance([r, g, b]: [number, number, number]): number {
  const lin = (c: number) => {
    c /= 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

export interface TileColors {
  /** Tile fill. */
  bg: string;
  /** Icon color on the tile: a deep shade of the same hue, or white on dark fills. */
  ink: string;
  /** Rings, glows and the ambient tint. */
  strong: string;
}

const tileCache = new Map<string, TileColors>();

export function tileColors(color: string): TileColors {
  const cached = tileCache.get(color);
  if (cached) return cached;
  const hex = colorHex(color);
  const c = rgb(hex);
  const [h, s] = toHsl(c);
  const light = luminance(c) > 0.32;
  const ink = light ? `hsl(${h.toFixed(0)} ${Math.min(62, s * 70).toFixed(0)}% 24%)` : "#ffffff";
  const out = { bg: hex, ink, strong: hex };
  tileCache.set(color, out);
  return out;
}

/** CSS custom properties for a tile, consumed by `.tile` and friends. */
export function tileVars(color: string): React.CSSProperties {
  const t = tileColors(color);
  return { "--tile": t.bg, "--tile-ink": t.ink } as React.CSSProperties;
}
