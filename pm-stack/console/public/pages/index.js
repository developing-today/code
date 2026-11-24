// The app's pages that are modules of their own.
//
// A page is one file in this folder that exports what it is:
//
//   export default {
//     route: "more/trends",        // where it lives: under More
//     title: "Trends",             // the heading, and its tile on More
//     group: "Plan",               // which part of More its tile is in
//     says: "…",                   // the line on its tile
//     view(sub, app) { … },        // the screen: a list of elements
//   };
//
// `view` is given what follows the route in the address (more/trends/30
// gives "30") and the app's own parts: `app.api(path)` reads (and the
// service worker keeps the answer for when there is no connection),
// `app.freshly(path)` marks a read as asked for again when the owner
// pressed refresh, `app.act(kind, args, said, button)` does something
// (kept on the device and sent when there is a connection, like every
// action), `app.go(route)`, `app.toast(text)`, `app.empty(text)`,
// `app.ago(iso)`. It builds with `h` from ../ui.js and the classes in
// app.css, and charts from ../charts.js.
//
// To add a page: write its file, add it to PAGES here, and name the
// file in ../files.json. The tests check all three agree, and the app
// gives it its route, its tile and its copy for offline.
import trends from "./trends.js";

export const PAGES = [trends];
