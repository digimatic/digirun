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

// Under --serve the route cards can be renamed; the server saves routes.txt and
// the page reloads on the same range, so the new name shows everywhere.
const RANGE_KEY = "digirun-range";
function rememberedRange() {
  try {
    const range = sessionStorage.getItem(RANGE_KEY);
    sessionStorage.removeItem(RANGE_KEY);
    return range;
  } catch {
    return null;
  }
}
if (body.dataset.editable !== undefined) {
  const range = rememberedRange();
  if (range) {
    setRange(range);
  }
  document.querySelectorAll(".route-card").forEach((card) => {
    const label = card.querySelector(".label");
    const row = document.createElement("div");
    row.className = "naming";
    label.replaceWith(row);
    const button = document.createElement("button");
    button.type = "button";
    button.className = "rename";
    button.textContent = "\u270e";
    button.title = "Rename this route";
    button.setAttribute("aria-label", `Rename ${label.textContent}`);
    row.append(label, button);
    const error = document.createElement("div");
    error.className = "rename-error";
    error.hidden = true;
    row.after(error);

    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const input = document.createElement("input");
      input.className = "rename-input";
      input.value = label.textContent;
      input.placeholder = "Empty: the automatic name";
      input.setAttribute("aria-label", "Route name");
      row.replaceChildren(input);
      input.focus();
      input.select();
      const restore = () => row.replaceChildren(label, button);
      // The card is a link: typing and clicking in the field must not follow it.
      input.addEventListener("click", (e) => {
        e.preventDefault();
        e.stopPropagation();
      });
      input.addEventListener("blur", restore);
      input.addEventListener("keydown", async (e) => {
        if (e.key === "Escape") {
          restore();
          return;
        }
        if (e.key !== "Enter") {
          return;
        }
        e.preventDefault();
        input.disabled = true;
        const form = new URLSearchParams({ route: card.dataset.route, name: input.value });
        try {
          const reply = await fetch("/route-name", {
            method: "POST",
            headers: { "X-Digirun": "rename" },
            body: form,
          });
          if (!reply.ok) {
            throw new Error((await reply.text()) || `HTTP ${reply.status}`);
          }
          try {
            sessionStorage.setItem(RANGE_KEY, body.dataset.range);
          } catch {
            // The page then comes back on its default range.
          }
          window.location.hash = "routes";
          window.location.reload();
        } catch (failure) {
          input.disabled = false;
          error.textContent = failure.message;
          error.hidden = false;
        }
      });
    });
  });
}
