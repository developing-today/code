/** Test helper: model the visible user's explicit choice for an unknown world. */
export function clickWorldJoin(root: HTMLElement): void {
  root.querySelector<HTMLButtonElement>("[data-world-join]")?.click();
  const notice = root.querySelector<HTMLElement>("[data-world-connect-notice]");
  if (notice && !notice.hidden) {
    root.querySelector<HTMLButtonElement>("[data-world-connect-once]")?.click();
  }
}
