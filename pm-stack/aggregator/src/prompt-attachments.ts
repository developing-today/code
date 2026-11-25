// Prompt attachments (#10036 S7, design #9539 §6): upload handling,
// multipart parsing, and storage refs for prompt submissions.
//
// Storage decision of record (design §8 question 3, answered 2026-11-22):
// blobs live in the pm-stack host data dir (alongside prompts.json), and
// records carry abstract storage refs — "attachment:<uuid>" — so a later
// move to Backblaze B2 (#9414) is a resolver migration, not a redesign.
// A ref names a blob file plus a JSON sidecar (original name, content
// type, size, sha256) inside the attachments directory; the ref itself
// never encodes a path, and resolution validates the ref's shape before
// touching the filesystem, so a forged ref cannot traverse out of the
// directory. Streaming audio is a future enhancement, out of scope here.
import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises";
import { join } from "node:path";

/** One attachment as carried on a prompt record (design §3.2: the
 *  record's "attachments" array holds these storage refs). */
export interface PromptAttachment {
  ref: string;
  name: string;
  contentType: string;
  size: number;
}

/** Sidecar metadata persisted next to each blob. */
export interface PromptAttachmentMeta extends PromptAttachment {
  sha256: string;
  createdAt: string;
}

export const MAX_ATTACHMENTS_PER_PROMPT = 5;
export const MAX_ATTACHMENT_BYTES = 5 * 1024 * 1024;
export const MAX_ATTACHMENTS_TOTAL_BYTES = 25 * 1024 * 1024;
/** Whole multipart request cap: the aggregate file cap plus room for the
 *  form fields and multipart framing. */
export const MAX_MULTIPART_BODY_BYTES = MAX_ATTACHMENTS_TOTAL_BYTES + 64 * 1024;

const REF_PATTERN = /^attachment:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function isPromptAttachmentRef(value: unknown): value is string {
  return typeof value === "string" && REF_PATTERN.test(value);
}

/** Display-name hygiene: the original filename is metadata only (blob
 *  paths derive from the uuid), but strip separators/control characters
 *  so it is safe to render and to echo in a Content-Disposition header. */
export function sanitizeAttachmentName(name: string): string {
  const cleaned = name
    .replace(/[/\\]/g, "_")
    .replace(/[\u0000-\u001f\u007f"]/g, "")
    .trim();
  return (cleaned === "" ? "attachment" : cleaned).slice(0, 200);
}

/** Keep only well-formed attachment entries (used when a prompts file is
 *  loaded back: a hand-edited or older record must not smuggle a ref the
 *  resolver would refuse, or a name that breaks rendering). */
export function normalizeAttachments(raw: unknown): PromptAttachment[] {
  if (!Array.isArray(raw)) return [];
  const out: PromptAttachment[] = [];
  for (const entry of raw) {
    if (typeof entry !== "object" || entry === null) continue;
    const e = entry as Record<string, unknown>;
    if (!isPromptAttachmentRef(e.ref)) continue;
    if (typeof e.size !== "number" || !Number.isFinite(e.size) || e.size < 0) continue;
    out.push({
      ref: e.ref,
      name: sanitizeAttachmentName(typeof e.name === "string" ? e.name : "attachment"),
      contentType:
        typeof e.contentType === "string" && e.contentType !== "" ? e.contentType : "application/octet-stream",
      size: e.size,
    });
    if (out.length >= MAX_ATTACHMENTS_PER_PROMPT) break;
  }
  return out;
}

export interface UploadedFile {
  name: string;
  contentType: string;
  bytes: Uint8Array;
}

export class PromptAttachmentsStore {
  private writeSeq = 0;

  constructor(private readonly dir: string) {}

  private blobPath(uuid: string): string {
    return join(this.dir, uuid);
  }

  private sidecarPath(uuid: string): string {
    return join(this.dir, `${uuid}.json`);
  }

  private uuidOf(ref: string): string | null {
    return isPromptAttachmentRef(ref) ? ref.slice("attachment:".length) : null;
  }

  /** Persist one uploaded file and return the record entry for it. The
   *  blob and its sidecar are each written temp-then-rename, like the
   *  other file-backed stores in this package. */
  async save(file: UploadedFile, now = new Date()): Promise<PromptAttachment> {
    const uuid = randomUUID();
    const meta: PromptAttachmentMeta = {
      ref: `attachment:${uuid}`,
      name: sanitizeAttachmentName(file.name),
      contentType: file.contentType === "" ? "application/octet-stream" : file.contentType,
      size: file.bytes.byteLength,
      sha256: createHash("sha256").update(file.bytes).digest("hex"),
      createdAt: now.toISOString(),
    };
    await mkdir(this.dir, { recursive: true });
    const blobTmp = `${this.blobPath(uuid)}.tmp-${process.pid}-${++this.writeSeq}`;
    await writeFile(blobTmp, file.bytes);
    await rename(blobTmp, this.blobPath(uuid));
    const sidecarTmp = `${this.sidecarPath(uuid)}.tmp-${process.pid}-${++this.writeSeq}`;
    await writeFile(sidecarTmp, JSON.stringify(meta, null, 2));
    await rename(sidecarTmp, this.sidecarPath(uuid));
    return { ref: meta.ref, name: meta.name, contentType: meta.contentType, size: meta.size };
  }

  /** Persist a whole submission's files, enforcing the count and size
   *  limits before anything is written, so a refused upload leaves no
   *  partial set behind. */
  async saveAll(files: UploadedFile[]): Promise<PromptAttachment[]> {
    if (files.length > MAX_ATTACHMENTS_PER_PROMPT) {
      throw new Error(`too many attachments: ${files.length} exceeds ${MAX_ATTACHMENTS_PER_PROMPT}`);
    }
    let total = 0;
    for (const file of files) {
      if (file.bytes.byteLength > MAX_ATTACHMENT_BYTES) {
        throw new Error(`attachment "${sanitizeAttachmentName(file.name)}" exceeds ${MAX_ATTACHMENT_BYTES} bytes`);
      }
      total += file.bytes.byteLength;
    }
    if (total > MAX_ATTACHMENTS_TOTAL_BYTES) {
      throw new Error(`attachments exceed ${MAX_ATTACHMENTS_TOTAL_BYTES} bytes in total`);
    }
    const saved: PromptAttachment[] = [];
    try {
      for (const file of files) saved.push(await this.save(file));
    } catch (err) {
      for (const entry of saved) await this.remove(entry.ref).catch(() => {});
      throw err;
    }
    return saved;
  }

  async read(ref: string): Promise<{ meta: PromptAttachmentMeta; bytes: Uint8Array<ArrayBuffer> } | null> {
    const uuid = this.uuidOf(ref);
    if (!uuid) return null;
    try {
      const meta = JSON.parse(await readFile(this.sidecarPath(uuid), "utf8")) as PromptAttachmentMeta;
      if (meta.ref !== ref) return null;
      // Copied onto a fresh ArrayBuffer so the bytes are a valid
      // Response BodyInit under the package's typings.
      const bytes = new Uint8Array(await readFile(this.blobPath(uuid)));
      return { meta, bytes };
    } catch {
      return null;
    }
  }

  async remove(ref: string): Promise<void> {
    const uuid = this.uuidOf(ref);
    if (!uuid) return;
    await unlink(this.blobPath(uuid)).catch(() => {});
    await unlink(this.sidecarPath(uuid)).catch(() => {});
  }
}

/** Read a multipart request body under the body cap and parse it. The
 *  bounded read mirrors readJsonBody's contract: an announced-oversize
 *  body is refused before it is pulled in. */
export async function readMultipartForm(req: Request): Promise<FormData> {
  const declaredLength = req.headers.get("content-length");
  if (declaredLength !== null && Number(declaredLength) > MAX_MULTIPART_BODY_BYTES) {
    await req.body?.cancel().catch(() => {});
    throw new Error("body too large");
  }
  const reader = req.body?.getReader();
  const chunks: Uint8Array[] = [];
  let byteLength = 0;
  if (reader) {
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        byteLength += value.byteLength;
        if (byteLength > MAX_MULTIPART_BODY_BYTES) {
          await reader.cancel().catch(() => {});
          throw new Error("body too large");
        }
        chunks.push(value);
      }
    } finally {
      reader.releaseLock();
    }
  }
  const bytes = new Uint8Array(byteLength);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  const reparsed = new Request("http://prompt-attachments.invalid/", {
    method: "POST",
    headers: { "content-type": req.headers.get("content-type") ?? "multipart/form-data" },
    body: bytes,
  });
  return reparsed.formData();
}

const SUBMISSION_FIELDS = ["scope", "target", "mode", "page", "sectionRef", "prompt", "replyTo", "variant"] as const;

/** Shape a multipart form's text fields into the JSON body
 *  validateNewPrompt already accepts, so multipart and JSON submissions
 *  go through exactly one validator. `notify` arrives as a form string
 *  and `refs` as a JSON-encoded array string. */
export function submissionFieldsFromForm(form: FormData): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const field of SUBMISSION_FIELDS) {
    const value = form.get(field);
    if (typeof value === "string") out[field] = value;
  }
  const notify = form.get("notify");
  if (typeof notify === "string") {
    if (notify === "true" || notify === "on" || notify === "1") out.notify = true;
    else if (notify === "false" || notify === "off" || notify === "0") out.notify = false;
    else out.notify = notify; // let the validator reject it with its own message
  }
  const refs = form.get("refs");
  if (typeof refs === "string" && refs.trim() !== "") {
    try {
      out.refs = JSON.parse(refs) as unknown;
    } catch {
      throw new Error("refs must be a JSON array of prompt ids");
    }
  }
  return out;
}

/** The uploaded files carried by a form, under the canonical
 *  "attachments" field name. An untouched file input submits one empty
 *  part (no name, no bytes); that is "no attachment", not a file. */
export async function uploadedFilesFromForm(form: FormData): Promise<UploadedFile[]> {
  const files: UploadedFile[] = [];
  for (const part of form.getAll("attachments")) {
    if (typeof part === "string") continue;
    const file = part as File;
    if (file.name === "" && file.size === 0) continue;
    files.push({
      name: file.name,
      contentType: file.type,
      bytes: new Uint8Array(await file.arrayBuffer()),
    });
  }
  return files;
}
