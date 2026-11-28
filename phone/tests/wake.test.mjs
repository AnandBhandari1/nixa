import test from "node:test";
import assert from "node:assert/strict";
import { WAKE_CALL_MS, hasName, createWake } from "../wake.js";

test("name match is a whole word", () => {
  assert.equal(hasName("Hey Nixa, are you there?"), true);
  assert.equal(hasName("nixavan says hi"), false);
  assert.equal(hasName("hello nova", "Nova"), true);
});

test("off hears nothing", () => {
  const wake = createWake();
  assert.equal(wake.state(), "off");
  assert.equal(wake.hear("nixa stop"), "");
});

test("name opens a call window that follow up lines extend", () => {
  let now = 0;
  const wake = createWake({ now: () => now });
  wake.setOn(true);
  assert.equal(wake.state(), "listening");
  assert.equal(wake.hear("what time is it"), "");
  assert.equal(wake.hear("nixa what time is it"), "nixa what time is it");
  assert.equal(wake.state(), "active");
  now = WAKE_CALL_MS - 1;
  assert.equal(wake.hear("and the date"), "and the date");
  now = WAKE_CALL_MS * 2 - 2;
  assert.equal(wake.hear("and the date"), "and the date");
  now = WAKE_CALL_MS * 4;
  assert.equal(wake.state(), "listening");
  assert.equal(wake.hear("and the date"), "");
  wake.setOn(false);
  assert.equal(wake.state(), "off");
});
