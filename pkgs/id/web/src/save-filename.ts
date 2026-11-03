/** Filename resolution for the editor save path. */

/**
 * Resolve the filename to post to /api/save from the editor container's
 * dataset.
 *
 * The server emits two name fields on #editor-container
 * (pkgs/id/src/web/templates.rs): `data-doc-id` carries the RAW filename
 * (html-escaped only), while `data-filename` carries
 * `urlencoding::encode(name)`. Every editor consumer reads
 * `dataset.filename` and decodes it.
 *
 * saveFile used to read `dataset.docId` and run decodeURIComponent on the
 * raw name (#1494): a filename containing a valid %XX sequence saved under
 * a DIFFERENT file's name (the server trusts the posted name, so this was
 * a cross-file overwrite), and a filename with a bare % threw URIError on
 * every save attempt, so the file could never be saved.
 */
export function resolveSaveFilename(dataset: { filename?: string }): string | null {
  const filenameEncoded = dataset.filename;
  return filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
}
