import test from "node:test";
import assert from "node:assert/strict";
import { SILENCE_MS, SILENCE_CHOICES, joinResults, createQuietWatch } from "../silence.js";

test("choices hold the default", () => {
  assert.ok(SILENCE_CHOICES.includes(SILENCE_MS));
});

test("join skips empty parts", () => {
  assert.equal(joinResults([" hi ", "", null, "nixa"]), "hi nixa");
  assert.equal(joinResults(["a", "b", "c"], 1), "b c");
});

test("quiet only after speech and the wait", () => {
  let now = 0;
  const watch = createQuietWatch({ ms: () => 1000, now: () => now });
  assert.equal(watch.quiet(), false);
  watch.sample(0.01);
  now = 5000;
  assert.equal(watch.quiet(), false);
  watch.sample(0.5);
  now = 5999;
  assert.equal(watch.quiet(), false);
  now = 6000;
  assert.equal(watch.quiet(), true);
  watch.reset();
  assert.equal(watch.quiet(), false);
});
