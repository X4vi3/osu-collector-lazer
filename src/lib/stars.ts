// osu!lazer's star-rating colour spectrum (OsuColour.ForStarDifficulty).
const STOPS: [number, string][] = [
  [0.1, "#aaaaaa"],
  [0.1, "#4290fb"],
  [1.25, "#4fc0ff"],
  [2.0, "#4fffd5"],
  [2.5, "#7cff4f"],
  [3.3, "#f6f05c"],
  [4.2, "#ff8068"],
  [4.9, "#ff4e6f"],
  [5.8, "#c645b8"],
  [6.7, "#6563de"],
  [7.7, "#18158e"],
  [9.0, "#000000"],
];

function hexToRgb(h: string): [number, number, number] {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function mix(a: string, b: string, t: number): string {
  const [r1, g1, b1] = hexToRgb(a);
  const [r2, g2, b2] = hexToRgb(b);
  const c = (x: number, y: number) => Math.round(x + (y - x) * t);
  return `rgb(${c(r1, r2)}, ${c(g1, g2)}, ${c(b1, b2)})`;
}

export function starColor(sr: number): string {
  if (sr < 0.1) return STOPS[0][1];
  for (let i = 1; i < STOPS.length; i++) {
    const [x1, c1] = STOPS[i - 1];
    const [x2, c2] = STOPS[i];
    if (sr <= x2) return x2 === x1 ? c2 : mix(c1, c2, (sr - x1) / (x2 - x1));
  }
  return STOPS[STOPS.length - 1][1];
}

/** Colour representing a whole-star bucket (its midpoint). */
export function bucketColor(star: number): string {
  return starColor(star >= 10 ? 10 : star + 0.5);
}

/** lazer switches the star text to yellow on very dark badges. */
export function starTextColor(sr: number): string {
  return sr >= 6.5 ? "#ffd966" : "#000000cc";
}

export function bucketLabel(star: number): string {
  return star >= 10 ? "10★+" : `${star}★`;
}
