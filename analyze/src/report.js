"use strict";

// The range toggle: the page holds a copy of each section per range.
const body = document.body;
const rangeButtons = document.querySelectorAll("[data-set-range]");
// On a narrow screen a chart scrolls sideways, so start at the newest data.
function showNewestData() {
  document.querySelectorAll(".plot").forEach((plot) => {
    plot.scrollLeft = plot.scrollWidth;
  });
}
function setRange(range) {
  body.dataset.range = range;
  rangeButtons.forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.setRange === range));
  });
  showNewestData();
}
rangeButtons.forEach((button) => {
  button.addEventListener("click", () => setRange(button.dataset.setRange));
});
showNewestData();

// One tooltip, filled from the mark under the pointer.
const tip = document.getElementById("tip");
document.addEventListener("pointermove", (event) => {
  const text = event.target.dataset ? event.target.dataset.tip : undefined;
  if (text === undefined) {
    tip.hidden = true;
    return;
  }
  tip.textContent = text;
  tip.hidden = false;
  const box = tip.getBoundingClientRect();
  const left = Math.min(event.clientX + 14, window.innerWidth - box.width - 8);
  const top = event.clientY - box.height - 12;
  tip.style.left = `${Math.max(8, left)}px`;
  tip.style.top = `${top < 8 ? event.clientY + 16 : top}px`;
});

// Expanding a session shows its passes.
const table = document.getElementById("sessions-table");
function setExpanded(group, expanded) {
  group.querySelector("tr.passes").hidden = !expanded;
  group.querySelector(".expand").setAttribute("aria-expanded", String(expanded));
}
table.addEventListener("click", (event) => {
  const row = event.target.closest("tr.session");
  if (row) {
    setExpanded(row.parentElement, row.nextElementSibling.hidden);
  }
});

// Sorting moves whole tbodies, so a session keeps its passes. Blanks stay last.
const groups = () => Array.from(table.querySelectorAll(":scope > tbody"));
table.querySelectorAll("th button").forEach((button) => {
  button.addEventListener("click", () => {
    const column = Number(button.dataset.column);
    const numeric = button.dataset.kind === "num";
    const header = button.parentElement;
    const ascending = header.getAttribute("aria-sort") !== "ascending";
    table.querySelectorAll("th").forEach((th) => th.removeAttribute("aria-sort"));
    header.setAttribute("aria-sort", ascending ? "ascending" : "descending");
    const key = (group) => {
      const cell = group.querySelector("tr.session").children[column];
      const value = cell.dataset.v === undefined ? cell.textContent : cell.dataset.v;
      if (!numeric) {
        return value;
      }
      return cell.dataset.v === undefined ? null : Number(value);
    };
    const sorted = groups().sort((a, b) => {
      const [x, y] = [key(a), key(b)];
      if (x === null || y === null) {
        return (x === null) - (y === null);
      }
      const order = numeric ? x - y : x.localeCompare(y);
      return ascending ? order : -order;
    });
    sorted.forEach((group) => table.appendChild(group));
  });
});

const routeFilter = document.getElementById("route-filter");
function filterRoute() {
  groups().forEach((group) => {
    group.hidden = routeFilter.value !== "" && group.dataset.route !== routeFilter.value;
  });
}
routeFilter.addEventListener("change", filterRoute);

// A route card links to the sessions, showing only that route's.
document.querySelectorAll(".route-card").forEach((card) => {
  card.addEventListener("click", () => {
    routeFilter.value = card.dataset.route;
    filterRoute();
  });
});

// A link to a session opens it, clearing whatever would hide it.
function openLinkedSession() {
  const id = window.location.hash.slice(1);
  if (!id.startsWith("s-")) {
    return;
  }
  const group = document.getElementById(id);
  if (group) {
    routeFilter.value = "";
    filterRoute();
    if (group.classList.contains("old")) {
      setRange("all");
    }
    setExpanded(group, true);
    group.scrollIntoView();
  }
}
window.addEventListener("hashchange", openLinkedSession);
openLinkedSession();
