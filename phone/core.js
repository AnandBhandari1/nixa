const TAU = Math.PI * 2;
const BARS = 72;
const TICKS = 120;
const GLOW = [[24, 0.04], [15, 0.07], [9, 0.13], [5, 0.26], [2, 1]];
const PINK_HUE = 330;
const ROSE_HUE = 345;

export const THEME = {
  background: "#0f1117",
  surface: "#161923",
  border: "#2a2f3d",
  foreground: "#c0caf5",
  dim: "#565f89",
  muted: "#737aa2",
  ansi1: "#f7768e",
  ansi2: "#9ece6a",
  ansi3: "#e0af68",
  ansi4: "#7aa2f7",
  ansi5: "#bb9af7",
  ansi6: "#7dcfff",
  ansi13: "#bb9af7",
};

export const LOOKS = {
  off: { word: "OFFLINE", sub: "Nixa is off on the Mac" },
  wait: { word: "CONNECTING", sub: "connecting to Nixa on the Mac" },
  you: { word: "HEARING YOU", sub: "" },
  nixa: { word: "SPEAKING", sub: "" },
  silent: { word: "NO SOUND", sub: "tap anywhere to hear Nixa" },
  muted: { word: "MUTED", sub: "Nixa speaks, sound is off here" },
  think: { word: "THINKING", sub: "" },
  paused: { word: "PAUSED", sub: "" },
  listen: { word: "LISTENING", sub: "" },
  standby: { word: "STANDBY", sub: "" },
  micoff: { word: "MIC OFF", sub: "the mic on the Mac is off" },
  ready: { word: "READY", sub: "hold to talk" },
};

export const ANIMATED = new Set(["you", "nixa", "think"]);

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

const parse = (hex) => {
  const m = /^#?([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(String(hex ?? "").trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
};

const toHex = (rgb) => `#${rgb.map((v) => Math.round(clamp(v, 0, 255)).toString(16).padStart(2, "0")).join("")}`;

const toHsl = ([r8, g8, b8]) => {
  const [r, g, b] = [r8 / 255, g8 / 255, b8 / 255];
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, s, l];
};

const fromHsl = (h, s, l) => {
  const k = (n) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n) => l - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1));
  return [f(0) * 255, f(8) * 255, f(4) * 255];
};

const pinkFamily = (t) => {
  const base = parse(t.ansi13) ?? parse(t.ansi5) ?? parse(t.ansi1);
  if (!base) return [t.ansi5, t.ansi1];
  const [, s, l] = toHsl(base);
  const sat = clamp(s, 0.55, 0.9);
  const light = clamp(l, 0.62, 0.74);
  return [toHex(fromHsl(PINK_HUE, sat, light)), toHex(fromHsl(ROSE_HUE, sat, light + 0.08))];
};

export const paletteOf = (mode, t = THEME) => {
  const [pink, rose] = pinkFamily(t);
  const sets = {
    off: [t.dim, t.muted, t.dim],
    wait: [t.ansi4, t.ansi6, t.ansi5],
    you: [t.ansi2, t.ansi6, t.ansi3],
    nixa: [pink, t.ansi5, rose],
    silent: [t.ansi3, t.ansi1, t.dim],
    muted: [t.dim, t.muted, t.dim],
    think: [t.ansi5, t.ansi4, pink],
    paused: [t.ansi3, t.ansi1, t.ansi4],
    listen: [t.ansi4, t.ansi6, t.ansi2],
    standby: [t.ansi4, t.ansi5, t.ansi6],
  };
  const [main, second, third] = sets[mode] ?? sets.standby;
  return { main, second, third };
};

const at = (c, r, a) => [c + r * Math.cos(a), c + r * Math.sin(a)];

const stroke = (ctx, width, color, alpha, dash, draw) => {
  ctx.save();
  ctx.globalAlpha = clamp(alpha, 0, 1);
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = "butt";
  if (dash) ctx.setLineDash(dash);
  ctx.beginPath();
  draw();
  ctx.stroke();
  ctx.restore();
};

const ring = (ctx, c, r, width, color, alpha, from = 0, dash = null) =>
  stroke(ctx, width, color, alpha, dash, () => ctx.arc(c, c, r, from, from + TAU));

const disk = (ctx, c, r, color, alpha) => {
  ctx.save();
  ctx.globalAlpha = clamp(alpha, 0, 1);
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(c, c, r, 0, TAU);
  ctx.fill();
  ctx.restore();
};

const voiceAmp = (mode, i, t, level) => {
  const half = BARS / 2;
  const k = i < half ? i : BARS - i;
  if (mode === "you") {
    const shape = 0.55 + 0.45 * Math.abs(Math.sin(k * 0.61 + t * 5.3)) * (0.7 + 0.3 * Math.sin(k * 0.23 - t * 2.1));
    return level * shape;
  }
  if (mode === "nixa") {
    const voice = 0.42 + 0.22 * Math.sin(t * 8.7) + 0.18 * Math.sin(t * 21.3 + 1.1) + 0.1 * Math.sin(t * 3.1);
    const shape = 0.5 + 0.5 * Math.abs(Math.sin(k * 0.37 + t * 6.2));
    return clamp(voice, 0, 1) * shape;
  }
  if (mode === "think") return 0.12 + 0.1 * Math.max(0, Math.sin(k * 0.5 - t * 7));
  return 0;
};

const bars = (ctx, c, inner, length, mode, t, level, palette) => {
  const shift = t * 0.08;
  palette.forEach((color, slotWanted) => stroke(ctx, 2.4, color, 0.9, null, () => {
    for (let i = 0; i < BARS; i++) {
      const amp = voiceAmp(mode, i, t, level);
      if (amp < 0.02) continue;
      const slot = Math.floor(((i / BARS + shift) % 1) * palette.length * 2) % palette.length;
      if (slot !== slotWanted) continue;
      const a = (TAU * i) / BARS - Math.PI / 2;
      ctx.moveTo(...at(c, inner, a));
      ctx.lineTo(...at(c, inner + 3 + amp * length, a));
    }
  }));
};

const ticks = (ctx, c, r, turn, color, alpha) => stroke(ctx, 1.2, color, alpha, null, () => {
  for (let i = 0; i < TICKS; i++) {
    const a = (TAU * i) / TICKS + turn;
    const long = i % 10 === 0;
    ctx.moveTo(...at(c, r, a));
    ctx.lineTo(...at(c, r - (long ? 10 : 4), a));
  }
});

const segments = (ctx, c, r, turn, count, span, color, alpha) => stroke(ctx, 3, color, alpha, null, () => {
  for (let i = 0; i < count; i++) {
    const from = turn + (TAU * i) / count;
    ctx.moveTo(...at(c, r, from));
    ctx.arc(c, c, r, from, from + span);
  }
});

const meter = (level, width = 18) => {
  const filled = Math.round(clamp(level, 0, 1) * width);
  return `▕${"█".repeat(filled)}${"░".repeat(width - filled)}▏`;
};

const text = (ctx, value, x, y, size, color, bold = false, align = "center") => {
  ctx.save();
  ctx.fillStyle = color;
  ctx.font = `${bold ? "700 " : ""}${size}px ui-monospace, Menlo, monospace`;
  ctx.textAlign = align;
  ctx.textBaseline = "middle";
  ctx.fillText(value, x, y);
  ctx.restore();
};

export const drawCore = (ctx, { size, t, mode, level, word, sub, theme = THEME }) => {
  const tones = paletteOf(mode, theme);
  const color = tones.main;
  const colors = { dash: tones.second, inner: tones.third, ticks: tones.third, label: tones.second, hint: theme.ansi5, halo: tones.second, bars: [tones.main, tones.second, tones.third] };
  const c = size / 2;
  const r0 = c - 6;
  const r1 = c - 26;
  const rGlow = size * 0.33 * (1 + 0.05 * level);
  const rDash = size * 0.245;
  const rDisk = size * 0.215;
  const spin = mode === "think" ? 1.9 : mode === "off" ? 0 : 0.35;
  const drift = mode === "off" ? 0 : 0.05;
  const strength = mode === "off" ? 0.4 : 0.55 + level * 0.9;
  const breath = ANIMATED.has(mode) ? 1 : 0.725 + 0.275 * Math.sin((t * TAU) / 2.6);

  ctx.clearRect(0, 0, size, size);
  disk(ctx, c, rDisk, color, 0.07 + level * 0.12);
  ring(ctx, c, rDash, 1, colors.dash, 0.7, -t * spin * 0.6, [3, 6]);
  ticks(ctx, c, r0, t * drift, colors.ticks, 0.55);
  segments(ctx, c, r1, -t * spin, 3, 0.95, color, 0.85);
  segments(ctx, c, r1 - 9, t * spin * 0.7, 5, 0.32, colors.inner, 0.6);
  for (const [width, alpha] of GLOW) ring(ctx, c, rGlow, width, width > 12 ? colors.halo : color, (width > 2 ? alpha * strength : 1) * breath);
  bars(ctx, c, rGlow + 14, size * 0.12, mode, t, level, colors.bars);

  const small = Math.max(11, Math.round(size * 0.034));
  text(ctx, "Nixa", c, c - small * 1.6, Math.round(size * 0.058), color, true);
  text(ctx, `· ${word} ·`, c, c + small * 0.4, small, colors.label);
  text(ctx, mode === "off" || mode === "wait" ? sub : meter(level), c, c + small * 1.9, Math.max(10, small - 2), colors.hint);
  if (mode !== "off" && mode !== "wait" && sub) text(ctx, sub, c, c + small * 3.2, Math.max(10, small - 2), colors.hint);
  text(ctx, "┌──", 0, 8, 12, colors.ticks, false, "left");
  text(ctx, "──┐", size, 8, 12, colors.ticks, false, "right");
  text(ctx, "└──", 0, size - 8, 12, colors.ticks, false, "left");
  text(ctx, "──┘", size, size - 8, 12, colors.ticks, false, "right");
};
