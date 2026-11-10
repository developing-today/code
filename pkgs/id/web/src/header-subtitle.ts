/**
 * Header subtitle rendering.
 *
 * Shows "p2p file sharing" on initial load, or the last-opened document's
 * filename as a link after navigation.
 *
 * The filename is untrusted document metadata (it arrives from the
 * server's URL-encoded data-filename attribute, decoded back to the raw
 * name in main.ts), so the link is built with DOM APIs: the name goes in
 * via textContent and the href via setAttribute, never string-interpolated
 * into innerHTML. A crafted name carrying markup (for example an img tag
 * with an onerror handler) therefore renders as literal text instead of
 * live markup (#1457). The href is set as an attribute (not the href
 * property) so the SPA router's getAttribute("href") read keeps the exact
 * path string it has always navigated to.
 */
export function updateHeaderSubtitle(
  lastFilename: string | null,
  lastFilePath: string | null,
  hasHistory: boolean,
): void {
  const subtitle = document.getElementById("header-subtitle");
  if (!subtitle) return;

  if (lastFilename && lastFilePath && hasHistory) {
    // Create a link to the last file
    const link = document.createElement("a");
    link.setAttribute("data-nav", "");
    link.setAttribute("href", lastFilePath);
    link.textContent = lastFilename;
    subtitle.replaceChildren(document.createTextNode("// "), link);
  } else {
    subtitle.textContent = "// p2p file sharing";
  }
}
