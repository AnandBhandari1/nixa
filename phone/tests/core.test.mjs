import test from "node:test";
import assert from "node:assert/strict";
import { LOOKS, ANIMATED, THEME, paletteOf, drawCore } from "../core.js";

const HEX = /^#[0-9a-f]{6}$/i;

test("every look has a word and a palette", () => {
  for (const [mode, look] of Object.entries(LOOKS)) {
    assert.ok(look.word.length > 0);
    const tones = paletteOf(mode);
    for (const tone of [tones.main, tones.second, tones.third]) assert.match(tone, HEX);
  }
  assert.ok(LOOKS.wait.sub.includes("Nixa"));
});

test("unknown mode uses standby colours", () => {
  assert.deepEqual(paletteOf("nope"), paletteOf("standby"));
});

test("speaking pink stays pink with a theme without ansi13", () => {
  const theme = { ...THEME, ansi13: undefined };
  assert.match(paletteOf("nixa", theme).main, HEX);
});

test("animated modes are looks", () => {
  for (const mode of ANIMATED) assert.ok(LOOKS[mode]);
});

test("draw runs on a fake canvas", () => {
  const calls = { text: [] };
  const ctx = new Proxy({}, {
    get: (target, key) => key in target ? target[key] : key === "fillText" ? (value) => calls.text.push(value) : () => {},
    set: (target, key, value) => { target[key] = value; return true; },
  });
  for (const mode of Object.keys(LOOKS)) drawCore(ctx, { size: 300, t: 1.5, mode, level: 0.5, word: LOOKS[mode].word, sub: LOOKS[mode].sub });
  assert.ok(calls.text.includes("Nixa"));
});
