import { test } from "node:test";
import assert from "node:assert/strict";
import { stashProof, takeProof } from "./proof-store.js";

test("a stashed proof is taken once, keyed by tool and args", () => {
  stashProof("graph.ask", { q: "MATCH (n)" }, "P1");
  assert.equal(takeProof("graph.ask", { q: "MATCH (n)" }), "P1");
  // Single use: a second take is undefined.
  assert.equal(takeProof("graph.ask", { q: "MATCH (n)" }), undefined);
});

test("the key is object-key-order independent", () => {
  stashProof("graph.write", { a: 1, b: 2 }, "P2");
  assert.equal(takeProof("graph.write", { b: 2, a: 1 }), "P2");
});

test("a different tool or args does not match", () => {
  stashProof("graph.ask", { q: "x" }, "P3");
  assert.equal(takeProof("graph.write", { q: "x" }), undefined);
  assert.equal(takeProof("graph.ask", { q: "y" }), undefined);
  // The original is still available.
  assert.equal(takeProof("graph.ask", { q: "x" }), "P3");
});

test("no proof is stashed when the gate minted none", () => {
  stashProof("graph.ask", { q: "z" }, undefined);
  assert.equal(takeProof("graph.ask", { q: "z" }), undefined);
});
