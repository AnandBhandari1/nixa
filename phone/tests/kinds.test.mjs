import test from "node:test";
import assert from "node:assert/strict";
import { KIND_LABELS, FALLBACK_KIND_LABEL, kindLabel, kindOf } from "../chat-kinds.js";

test("every kind has a short label", () => {
  for (const [kind, label] of Object.entries(KIND_LABELS)) {
    assert.equal(kindLabel(kind), label);
    assert.ok(label.length > 0 && label.length <= 12);
  }
});

test("unknown kind falls back", () => {
  assert.equal(kindLabel("nope"), FALLBACK_KIND_LABEL);
  assert.equal(kindLabel(undefined), FALLBACK_KIND_LABEL);
});

test("kind from who and text", () => {
  assert.equal(kindOf({ who: "user" }), "heard");
  assert.equal(kindOf({ who: "user", typed: true }), "typed");
  assert.equal(kindOf({ who: "nixa" }), "nixa");
  assert.equal(kindOf({ who: "agent" }), "agent");
  assert.equal(kindOf({ who: "agent", to: true }), "to");
  assert.equal(kindOf({ kind: "team", who: "nixa" }), "team");
  assert.equal(kindOf({ text: "Decision auto: go" }), "decision");
  assert.equal(kindOf({ text: "Debate started" }), "debate");
  assert.equal(kindOf({ text: "Waits for you: pick one" }), "question");
  assert.equal(kindOf({ text: "All checks passed" }), "check");
  assert.equal(kindOf({ text: "3 new in the inbox" }), "tasks");
  assert.equal(kindOf({ text: "The team is ready" }), "team");
  assert.equal(kindOf({ text: "m4 is done" }), "done");
  assert.equal(kindOf({ text: "Noted: tea" }), "note");
  assert.equal(kindOf({ text: "plain line" }), "info");
});
