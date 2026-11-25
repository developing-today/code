// The two things every screen is built with: an element, and a drawn
// (SVG) element. A screen is a list of elements made with these and the
// app's classes (app.css), so that one screen's card, chip, row or
// button is every other screen's.
//
// Nothing here writes a style: what an element looks like is its class.

/** An element. `attrs` are its attributes: `class` is its classes, a
 * name starting "on" is a listener, true is an attribute with no value,
 * and false, null or undefined is no attribute. `kids` are elements or
 * text, however nested; nothing (null, undefined, false) is left out. */
export function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v === null || v === undefined) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const SVG = "http://www.w3.org/2000/svg";

/** A drawn element (svg, g, rect, path, line, text…), made the same
 * way. Numbers are written to two places, which is finer than a screen. */
export function s(tag, attrs, ...kids) {
  const el = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v === null || v === undefined) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, typeof v === "number" ? String(Math.round(v * 100) / 100) : v === true ? "" : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}
