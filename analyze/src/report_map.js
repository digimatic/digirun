"use strict";

// The satellite maps, in the page only with --map. They start on, unless turned
// off earlier, and tiles are only requested for a map on screen.
(() => {
  const TILES =
    "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2025_3857/default/GoogleMapsCompatible/{z}/{y}/{x}.jpg";
  const CREDIT =
    'EOxCloudless <a href="https://cloudless.eox.at">https://cloudless.eox.at</a> by ' +
    '<a href="https://eox.at">EOX IT Services GmbH</a> (Contains modified Copernicus Sentinel data 2025)';
  const STORAGE_KEY = "digirun-map";

  const toggle = document.getElementById("map-toggle");
  const maps = new Map();
  let on = true;
  try {
    on = localStorage.getItem(STORAGE_KEY) !== "off";
  } catch {
    // Storage can be blocked; the map then just starts on.
  }

  const colour = (name) =>
    getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();

  // A casing keeps a line visible over any imagery.
  function drawLine(map, points, stroke, casing) {
    L.polyline(points, { color: casing, weight: 5, opacity: 0.7, interactive: false }).addTo(map);
    return L.polyline(points, { color: stroke, weight: 3 }).addTo(map);
  }

  function drawSession(map, passes) {
    const strokes = { 0: colour("s2"), 1: colour("s1"), 2: "#ffffff" };
    const all = [];
    passes.forEach(([kind, points]) => {
      if (points.length > 0) {
        drawLine(map, points, strokes[kind], "#ffffff");
        all.push(...points);
      }
    });
    if (all.length > 0) {
      L.circleMarker(all[0], { radius: 5, color: "#ffffff", weight: 2, fillColor: "#0b0b0b", fillOpacity: 1 }).addTo(map);
    }
    return all;
  }

  function drawRoutes(map, routes) {
    const near = [];
    routes.forEach((route) => {
      const coloured = route.mark !== "other";
      const line = coloured
        ? drawLine(map, route.track, colour(route.mark), "#ffffff")
        : drawLine(map, route.track, "#ffffff", "#000000");
      const tip = document.createElement("span");
      tip.textContent = route.tip;
      line.bindTooltip(tip, { sticky: true, className: "route-tip" });
      if (route.near) {
        near.push(...route.track);
      }
    });
    return near;
  }

  function makeMap(host) {
    const map = L.map(host, { scrollWheelZoom: false, maxZoom: 16 });
    L.tileLayer(TILES, { maxNativeZoom: 14, maxZoom: 16, attribution: CREDIT }).addTo(map);
    const points = host.dataset.track
      ? drawSession(map, JSON.parse(host.dataset.track))
      : drawRoutes(map, JSON.parse(host.dataset.routes));
    if (points.length > 0) {
      map.fitBounds(points, { padding: [16, 16] });
    } else {
      map.setView([0, 0], 1);
    }
    return map;
  }

  // A map can only measure itself while on screen, so each is made when first shown.
  function sync() {
    document.body.classList.toggle("map-on", on);
    toggle.setAttribute("aria-pressed", String(on));
    if (!on) {
      return;
    }
    document.querySelectorAll(".map-host").forEach((host) => {
      if (host.offsetParent === null) {
        return;
      }
      if (maps.has(host)) {
        maps.get(host).invalidateSize();
      } else {
        maps.set(host, makeMap(host));
      }
    });
  }

  toggle.addEventListener("click", () => {
    on = !on;
    try {
      localStorage.setItem(STORAGE_KEY, on ? "on" : "off");
    } catch {
      // Then the choice lasts until the page closes.
    }
    sync();
  });

  // Picking a range or expanding a session can bring a map on screen.
  new MutationObserver(sync).observe(document.body, { attributes: true, attributeFilter: ["data-range"] });
  new MutationObserver(sync).observe(document.getElementById("sessions-table"), {
    subtree: true,
    attributes: true,
    attributeFilter: ["hidden"],
  });
  sync();
})();
