// UI wiring. All scanning happens in Rust (pixelsurf-core); this only renders the result.
"use strict";

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const el = (t) => document.createElement(t);

// ---------------------------------------------------------------- tabs
for (const b of document.querySelectorAll(".tab")) {
  b.addEventListener("click", () => {
    for (const x of document.querySelectorAll(".tab")) x.classList.toggle("active", x === b);
    for (const p of document.querySelectorAll(".tab-panel")) {
      p.classList.toggle("active", p.id === "tab-" + b.dataset.tab);
    }
  });
}

// ---------------------------------------------------------------- map scan
let scanned = null;      // last result
let filter = "all";

async function loadMaps() {
  try {
    const list = await invoke("list_maps");
    const dl = $("maplist");
    dl.innerHTML = "";
    for (const m of list) { const o = el("option"); o.value = m; dl.appendChild(o); }
    const folders = await invoke("map_folders");
    setStatus(`${list.length} maps found in ${folders.length} folder${folders.length === 1 ? "" : "s"}. Pick one and hit Scan.`);
  } catch (e) {
    setStatus(String(e), true);
  }
}

function setStatus(msg, isErr) {
  const s = $("status");
  s.textContent = msg;
  s.classList.toggle("err", !!isErr);
}

async function doScan(force) {
  const map = $("map").value.trim().toLowerCase();
  if (!map) { setStatus("Type a map name first.", true); return; }
  $("scan").disabled = $("rescan").disabled = true;
  setStatus(force ? `Rescanning ${map}…` : `Scanning ${map}…`);
  $("results").classList.add("hidden");
  $("summary").classList.add("hidden");
  $("warn").classList.add("hidden");
  $("more").classList.add("hidden");
  try {
    const json = await invoke("scan", {
      map,
      includeGround: $("ground").checked,
      includeTrim: $("trim").checked,
      includeSurf: $("surf").checked,
      minOob: parseFloat($("minoob").value) || 0,
      force: !!force,
    });
    scanned = JSON.parse(json);
    filter = "all";
    render();
  } catch (e) {
    setStatus(String(e), true);
  } finally {
    $("scan").disabled = $("rescan").disabled = false;
  }
}

function render() {
  if (!scanned) return;
  const c = scanned.counts;
  const oob = scanned.spots.filter((s) => !s.reachable).length;
  setStatus(`${scanned.map} — scanned in ${scanned.scanMs} ms (bsp v${scanned.bspVersion}, ${scanned.stats.spawns} spawns). Cached; rescan only if the map file changed.`);

  const counts = { all: scanned.spots.length, oob, pixelsurf: c.pixelsurf,
    pixelwalk: c.pixelwalk, surf: c.surf };
  for (const chip of document.querySelectorAll(".chip")) {
    const k = chip.dataset.filter;
    chip.querySelector("b").textContent = counts[k] ?? 0;
    chip.classList.toggle("on", k === filter);
  }
  $("summary").classList.remove("hidden");

  if (scanned.limitations && scanned.limitations.length) {
    $("warn").innerHTML = "<b>Not scanned:</b><ul>" +
      scanned.limitations.map((l) => `<li>${escapeHtml(l)}</li>`).join("") + "</ul>";
    $("warn").classList.remove("hidden");
  }

  let rows = scanned.spots;
  if (filter === "oob") rows = rows.filter((s) => !s.reachable);
  else if (filter !== "all") rows = rows.filter((s) => s.kind === filter);

  const LIMIT = 300;
  const shown = rows.slice(0, LIMIT);
  $("rows").innerHTML = shown.map(rowHtml).join("");
  $("results").classList.toggle("hidden", shown.length === 0);
  if (rows.length > LIMIT) {
    $("more").textContent = `Showing the first ${LIMIT} of ${rows.length}. Narrow it with the filters above.`;
    $("more").classList.remove("hidden");
  } else {
    $("more").classList.add("hidden");
  }
  if (!shown.length) setStatus(`${scanned.map} — nothing matches this filter.`);
}

function rowHtml(s) {
  const e = s.entries && s.entries[0];
  const how = !e ? "<span class='dimtext'>—</span>"
    : `${escapeHtml(e.label)} @ <span class="eye">${e.standEye.toFixed(2)}</span>` +
      (e.jump === null ? " <span class='dimtext'>(walk off)</span>"
        : `, ${e.crouch ? "crouch " : ""}${e.jump.toFixed(2)}u`);
  const above = s.heightAboveReachable === null || s.heightAboveReachable === undefined
    ? "<span class='dimtext'>—</span>" : `<span class="oob">${s.heightAboveReachable.toFixed(0)}u</span>`;
  const why = s.oobClass ? `<span class="oob">${escapeHtml(s.oobClass)}</span>`
    : "<span class='dimtext'>reachable</span>";
  return `<tr>
    <td class="kind-${s.kind}">${s.kind}${s.isClip ? " <span class='dimtext'>(clip)</span>" : ""}</td>
    <td class="num">${s.x.toFixed(0)}</td><td class="num">${s.y.toFixed(0)}</td>
    <td class="num">${s.z.toFixed(1)}</td>
    <td class="num">${s.width.toFixed(1)}u</td>
    <td class="num">${s.reachable ? "<span class='dimtext'>—</span>" : above}</td>
    <td>${why}</td><td>${how}</td></tr>`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

$("scan").addEventListener("click", () => doScan(false));
$("rescan").addEventListener("click", () => doScan(true));
$("map").addEventListener("keydown", (e) => { if (e.key === "Enter") doScan(false); });
for (const chip of document.querySelectorAll(".chip")) {
  chip.addEventListener("click", () => { filter = chip.dataset.filter; render(); });
}

// ---------------------------------------------------------------- calculator tab
function renderCalc() {
  const ledge = parseFloat($("ledge").value);
  const lo = parseFloat($("lo").value), hi = parseFloat($("hi").value);
  const tick = parseInt($("tick").value, 10);
  const maxPlayers = parseInt($("players").value, 10);
  if (![ledge, lo, hi].every(Number.isFinite)) {
    $("calcrows").innerHTML = ""; $("calcsum").textContent = "Enter a ledge height and a range.";
    return;
  }
  const sols = PixelJump.solutions(ledge, Math.min(lo, hi), Math.max(lo, hi), tick, maxPlayers);
  $("calcsum").textContent =
    `Ledge ${ledge.toFixed(2)} — ${sols.length} way${sols.length === 1 ? "" : "s"} in.`;
  $("calcrows").innerHTML = sols.slice(0, 400).map((s) => `<tr>
    <td>${escapeHtml(s.label)}</td>
    <td class="num eye">${s.standEye.toFixed(2)}</td>
    <td>${s.jump === null ? "<span class='dimtext'>walk off the head</span>"
      : (s.crouch ? "crouch jump" : "normal jump")}</td>
    <td class="num">${s.jump === null ? "—" : s.jump.toFixed(2) + "u"}</td>
    <td>${s.t64 ? "64 and 128" : "<span class='dimtext'>128 only</span>"}</td></tr>`).join("");
}
for (const id of ["ledge", "lo", "hi", "tick", "players"]) {
  $(id).addEventListener("input", renderCalc);
  $(id).addEventListener("change", renderCalc);
}

renderCalc();
loadMaps();
