// Tiny test runner — no dependencies, so `npm test` works before any install.
"use strict";

const path = require("path");

let pass = 0, fail = 0;
const failures = [];

const t = {
  ok(cond, msg) { if (cond) pass++; else { fail++; failures.push(msg); } },
  eq(a, b, msg) { this.ok(Object.is(a, b) || a === b, `${msg} (got ${a}, want ${b})`); },
};

const SUITES = ["jumptable.test.js", "surfaces.test.js", "bspcollide.test.js"];

for (const s of SUITES) {
  let mod;
  try { mod = require(path.join(__dirname, s)); }
  catch (e) {
    if (e.code === "MODULE_NOT_FOUND" && e.message.includes(s.replace(".js", ""))) continue;
    if (e.code === "MODULE_NOT_FOUND") { console.log(`- ${s} skipped (${e.message.split("\n")[0]})`); continue; }
    throw e;
  }
  const before = pass + fail;
  try { mod(t); } catch (e) { fail++; failures.push(`${s} threw: ${e.message}`); }
  console.log(`  ${s}: ${pass + fail - before} assertions`);
}

console.log(`\n${pass} passed, ${fail} failed`);
if (failures.length) {
  console.log("\nfailures:");
  for (const f of failures.slice(0, 25)) console.log("  - " + f);
  process.exit(1);
}
