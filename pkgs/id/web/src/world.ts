/** Browser presentation for the server-authoritative world session. */

import { terminalBytesForKey } from "./world-keyboard";
import { parseWorldSeed } from "./world-seed";
import { applyScreenPatch, renderWorldSurface, type ScreenPatch, type ScreenSurface } from "./world-surface";

type WorldFrame = {
  type:
    | "snapshot"
    | "event"
    | "view"
    | "records"
    | "invite"
    | "upload_ready"
    | "module_installed"
    | "compiled"
    | "error";
  snapshot?: {
    world_id: string;
    current_sequence: number;
    active_module_hash?: string | null;
    participants: Array<{ id: number; display_name: string }>;
  };
  event?: {
    sequence: number;
    participant_id: number;
    kind: { kind: "chat" | "input"; data: string | number[] };
  };
  sequence?: number;
  data?: string;
  surface?: ScreenSurface;
  view_revision?: number;
  base_view_revision?: number;
  patch?: ScreenPatch;
  invite?: { capability?: string; participant_id?: number; display_name?: string };
  capability?: string;
  records?: Record<string, unknown>;
  next_after?: string | null;
  chunk_bytes?: number;
  module_hash?: string;
  diagnostics?: string;
  participant_id?: number;
  display_name?: string;
  message?: string;
};

function element<T extends HTMLElement>(root: HTMLElement, selector: string): T | null {
  return root.querySelector<T>(selector);
}

function appendLine(log: HTMLOListElement, text: string): void {
  const row = document.createElement("li");
  row.textContent = text;
  log.append(row);
  while (log.childElementCount > 200) log.firstElementChild?.remove();
  log.scrollTop = log.scrollHeight;
}

function initWorld(): void {
  const root = document.querySelector<HTMLElement>("[data-world-app]");
  if (!root || root.dataset.initialized === "true") return;
  root.dataset.initialized = "true";

  const name = element<HTMLInputElement>(root, "[data-world-name]");
  const admin = element<HTMLInputElement>(root, "[data-world-admin]");
  const capability = element<HTMLInputElement>(root, "[data-world-capability]");
  const inviteButton = element<HTMLButtonElement>(root, "[data-world-invite]");
  const moduleInput = element<HTMLInputElement>(root, "[data-world-module]");
  const sourceInput = element<HTMLInputElement>(root, "[data-world-source]");
  const moduleSeed = element<HTMLInputElement>(root, "[data-world-seed]");
  const installButton = element<HTMLButtonElement>(root, "[data-world-install]");
  const compileButton = element<HTMLButtonElement>(root, "[data-world-compile]");
  const joinButton = element<HTMLButtonElement>(root, "[data-world-join]");
  const hostViewButton = element<HTMLButtonElement>(root, "[data-world-host-view]");
  const chatForm = element<HTMLFormElement>(root, "[data-world-chat-form]");
  const chatInput = element<HTMLInputElement>(root, "[data-world-chat]");
  const inputForm = element<HTMLFormElement>(root, "[data-world-input-form]");
  const gameInput = element<HTMLInputElement>(root, "[data-world-input]");
  const log = element<HTMLOListElement>(root, "[data-world-log]");
  const view = element<HTMLElement>(root, "[data-world-view]");
  const participants = element<HTMLElement>(root, "[data-world-participants]");
  const status = element<HTMLElement>(root, "[data-world-status]");
  const worldInput = element<HTMLInputElement>(root, "[data-world-world]");
  const recordsView = element<HTMLElement>(root, "[data-world-records]");
  const recordsRefresh = element<HTMLButtonElement>(root, "[data-world-records-refresh]");
  const copyButton = element<HTMLButtonElement>(root, "[data-world-copy]");
  const inviteOutput = element<HTMLElement>(root, "[data-world-invite-output]");
  if (
    !name ||
    !admin ||
    !capability ||
    !worldInput ||
    !inviteButton ||
    !moduleInput ||
    !sourceInput ||
    !moduleSeed ||
    !installButton ||
    !compileButton ||
    !joinButton ||
    !hostViewButton ||
    !chatForm ||
    !chatInput ||
    !inputForm ||
    !gameInput ||
    !log ||
    !view ||
    !recordsView ||
    !recordsRefresh ||
    !participants ||
    !status ||
    !copyButton ||
    !inviteOutput
  )
    return;

  // `/world?world=arena` opens the page on that world.
  worldInput.value = new URLSearchParams(window.location.search).get("world") ?? "";
  // The world a request is addressed to; `null` means the server's default.
  const selectedWorld = (): string | null => worldInput.value.trim() || null;

  const storageKey = `id-world-capability:${window.location.host}`;
  capability.value = sessionStorage.getItem(storageKey) ?? "";
  let socket: WebSocket | null = null;
  let cursor = 0;
  let renderedSurface: ScreenSurface | null = null;
  let renderedViewRevision: number | null = null;
  let records: Record<string, unknown> = {};
  let recordsAfter: string | null = null;
  let recordsPending = false;
  let recordsRefetch = false;
  let reconnects = 0;
  let intentionalClose = false;
  let hostMode = false;
  let refreshHostView: () => Promise<void> = async () => {};

  const setStatus = (text: string, state: "idle" | "good" | "error" = "idle") => {
    status.textContent = text;
    status.dataset.state = state;
  };

  const showCapability = (value: string) => {
    capability.value = value;
    sessionStorage.setItem(storageKey, value);
    inviteOutput.hidden = false;
  };

  const receive = (raw: string) => {
    let frame: WorldFrame;
    try {
      frame = JSON.parse(raw) as WorldFrame;
    } catch {
      setStatus("Received an invalid world frame", "error");
      return;
    }
    switch (frame.type) {
      case "snapshot": {
        if (!frame.snapshot) return;
        cursor = frame.snapshot.current_sequence;
        participants.replaceChildren();
        for (const participant of frame.snapshot.participants) {
          const tag = document.createElement("span");
          tag.className = "badge badge-outline mr-2";
          tag.dataset.participant = String(participant.id);
          tag.textContent = participant.display_name;
          participants.append(tag);
        }
        appendLine(log, `Connected to ${frame.snapshot.world_id} at event ${cursor}`);
        setStatus("Connected", "good");
        if (!hostMode) requestRecords();
        break;
      }
      case "event": {
        const event = frame.event;
        if (!event) return;
        cursor = Math.max(cursor, event.sequence);
        if (event.kind.kind === "chat") {
          const who = participants.querySelector<HTMLElement>(`[data-participant="${event.participant_id}"]`);
          appendLine(log, `${who?.textContent ?? `player ${event.participant_id}`}: ${String(event.kind.data)}`);
        } else {
          appendLine(log, `input #${event.sequence} from player ${event.participant_id}`);
          requestRecords();
        }
        break;
      }
      case "view":
        if (frame.patch) {
          if (
            renderedSurface &&
            renderedViewRevision === frame.base_view_revision &&
            typeof frame.view_revision === "number"
          ) {
            const patched = applyScreenPatch(renderedSurface, frame.patch);
            if (patched && renderWorldSurface(view, patched)) {
              renderedSurface = patched;
              renderedViewRevision = frame.view_revision;
              break;
            }
          }
          // A gap or invalid patch means the browser no longer has the exact
          // base. A same-size resize asks the server to reset with a full view.
          sendViewport();
          break;
        }
        if (frame.surface && renderWorldSurface(view, frame.surface)) {
          renderedSurface = frame.surface;
          renderedViewRevision = frame.view_revision ?? null;
          break;
        }
        if (frame.data !== undefined) {
          view.textContent = frame.data;
          renderedSurface = null;
          renderedViewRevision = frame.view_revision ?? null;
        }
        break;
      case "records": {
        if (!recordsPending) return;
        Object.assign(records, frame.records ?? {});
        recordsAfter = frame.next_after ?? null;
        if (recordsAfter === null) {
          recordsPending = false;
          recordsView.textContent = JSON.stringify(records, null, 2);
          // #1458: a re-request arrived while this fetch was in flight
          // and was coalesced (see requestRecords). Now that a complete
          // set has rendered, run the refetch it was waiting for.
          if (recordsRefetch) {
            recordsRefetch = false;
            requestRecords();
          }
        } else {
          socket?.send(
            JSON.stringify({
              type: "records",
              capability: capability.value.trim(),
              after: recordsAfter,
            }),
          );
        }
        break;
      }
      case "invite": {
        const invite = frame.invite ?? frame;
        if (invite.capability) {
          showCapability(invite.capability);
          appendLine(log, `Invite created for ${invite.display_name ?? (name.value || "guest")}`);
          setStatus("Invite ready to share", "good");
        }
        break;
      }
      case "error":
        setStatus(frame.message ?? "World request failed", "error");
        appendLine(log, `error: ${frame.message ?? "world request failed"}`);
        break;
    }
  };

  const requestRecords = () => {
    if (hostMode) {
      void refreshHostView();
      return;
    }
    if (socket?.readyState !== WebSocket.OPEN) {
      setStatus("Join the world before reading records", "error");
      return;
    }
    // #1458: never overlap records fetches. Records pages carry no
    // fetch identity, so a first-page request sent mid-pagination lets
    // the stale chain's terminal page close the fresh fetch and render
    // as the result, while the fresh pages are then dropped by the
    // recordsPending guard. Coalesce instead: note that a refetch is
    // due and run it when the in-flight fetch completes.
    if (recordsPending) {
      recordsRefetch = true;
      return;
    }
    records = {};
    recordsAfter = null;
    recordsPending = true;
    // This session is joined, so it can also ask for records.
    socket.send(
      JSON.stringify({
        type: "records",
        capability: capability.value.trim(),
      }),
    );
  };

  const sendInputBytes = (bytes: Uint8Array) => {
    if (hostMode) {
      setStatus("Host view is read-only", "error");
      return false;
    }
    if (socket?.readyState !== WebSocket.OPEN) {
      setStatus("Join the world before sending game input", "error");
      return false;
    }
    const dataHex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
    socket.send(JSON.stringify({ type: "input", data_hex: dataHex }));
    return true;
  };

  view.addEventListener("keydown", (event: KeyboardEvent) => {
    const bytes = terminalBytesForKey(event);
    if (!bytes) return;
    if (sendInputBytes(bytes)) event.preventDefault();
  });
  view.addEventListener("paste", (event: ClipboardEvent) => {
    if (hostMode) {
      event.preventDefault();
      return;
    }
    const text = event.clipboardData?.getData("text/plain");
    if (text === undefined || !text) return;
    if (new TextEncoder().encode(text).length > 4096) {
      setStatus("Paste is limited to 4096 UTF-8 bytes", "error");
      event.preventDefault();
      return;
    }
    if (socket?.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: "paste", text }));
      event.preventDefault();
    }
  });

  const sendViewport = () => {
    if (hostMode || socket?.readyState !== WebSocket.OPEN) return;
    const fontSize = Number.parseFloat(getComputedStyle(view).fontSize) || 16;
    const width = Math.max(1, Math.min(500, Math.floor(view.clientWidth / (fontSize * 0.6))));
    const height = Math.max(1, Math.min(200, Math.floor(view.clientHeight / fontSize)));
    socket.send(JSON.stringify({ type: "resize", width, height }));
  };
  const viewportObserver = new ResizeObserver(sendViewport);
  viewportObserver.observe(view);

  recordsRefresh.addEventListener("click", requestRecords);

  refreshHostView = async () => {
    const secret = admin.value.trim();
    if (!secret) {
      setStatus("Enter the world admin token to view as host", "error");
      return;
    }
    if (socket) {
      intentionalClose = true;
      const retired = socket;
      socket = null;
      retired.close();
    }
    hostMode = true;
    chatForm.hidden = true;
    inputForm.hidden = true;
    view.tabIndex = -1;
    hostViewButton.disabled = true;
    setStatus("Loading read-only host view…");
    renderedSurface = null;
    renderedViewRevision = null;
    const world = selectedWorld();
    let after: string | null = null;
    let recordsSequence: number | null = null;
    const allRecords: Record<string, unknown> = {};
    let firstPage = true;
    try {
      do {
        const params = new URLSearchParams();
        if (world) params.set("world", world);
        if (after) params.set("after", after);
        const response = await fetch(`/api/world/host-view?${params}`, {
          headers: { "x-world-admin-token": secret },
          cache: "no-store",
        });
        if (!response.ok) {
          throw new Error(
            response.status === 401 ? "Host authorization denied" : "Host view unavailable",
          );
        }
        const result = (await response.json()) as {
          snapshot?: NonNullable<WorldFrame["snapshot"]>;
          view?: string | null;
          surface?: ScreenSurface | null;
          records_sequence: number;
          records: Record<string, unknown>;
          next_after: string | null;
        };
        if (recordsSequence !== null && recordsSequence !== result.records_sequence) {
          throw new Error("World records changed while paging; refresh the host view");
        }
        recordsSequence = result.records_sequence;
        Object.assign(allRecords, result.records);
        if (firstPage) {
          firstPage = false;
          cursor = result.snapshot?.current_sequence ?? 0;
          participants.replaceChildren();
          for (const participant of result.snapshot?.participants ?? []) {
            const tag = document.createElement("span");
            tag.className = "badge badge-outline mr-2";
            tag.textContent = participant.display_name;
            participants.append(tag);
          }
          renderedSurface = null;
          renderedViewRevision = null;
          if (result.surface && renderWorldSurface(view, result.surface)) {
            renderedSurface = result.surface;
          } else if (result.view) {
            view.textContent = result.view;
          } else if (!result.snapshot?.active_module_hash) {
            view.textContent = "No program is installed. Use Install or update program below.";
          } else {
            view.textContent = "The active program has no text view.";
          }
          appendLine(log, `Viewing ${result.snapshot?.world_id ?? world ?? "default world"} as host (read-only)`);
        }
        after = result.next_after;
      } while (after !== null);
      recordsView.textContent = JSON.stringify(allRecords, null, 2);
      setStatus("Read-only host view", "good");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Host view unavailable", "error");
    } finally {
      hostViewButton.disabled = false;
    }
  };
  hostViewButton.addEventListener("click", () => void refreshHostView());

  const connect = () => {
    const token = capability.value.trim();
    if (!token) {
      setStatus("Create or paste a guest capability first", "error");
      return;
    }
    intentionalClose = false;
    // A new connection owns no records fetch: one left pending by a
    // previous connection can never complete and must not coalesce
    // (block) this connection's first fetch.
    recordsPending = false;
    recordsRefetch = false;
    renderedSurface = null;
    renderedViewRevision = null;
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    // #1461: every handler captures THIS connection's socket (`ws`) and
    // ignores events once it is no longer the current one. The handlers
    // used to read the shared mutable `socket`, so a retired socket's
    // async close event (dispatched after a rejoin had already replaced
    // it) scheduled a duplicate reconnect, and a stale socket's open
    // sent its join frame on the newer socket.
    const ws = new WebSocket(`${scheme}//${location.host}/ws/world`);
    socket = ws;
    setStatus("Connecting…");
    ws.addEventListener("open", () => {
      if (ws !== socket) return;
      reconnects = 0;
      ws.send(
        JSON.stringify({
          type: "join",
          capability: token,
          after: cursor || null,
          world: selectedWorld(),
          screen_patches: true,
        }),
      );
      sendViewport();
    });
    ws.addEventListener("message", (event) => {
      if (ws !== socket) return;
      receive(String(event.data));
    });
    ws.addEventListener("error", () => {
      if (ws !== socket) return;
      setStatus("World connection failed", "error");
    });
    ws.addEventListener("close", () => {
      if (ws !== socket) return;
      socket = null;
      // This connection's records fetch, if any, can never complete.
      recordsPending = false;
      recordsRefetch = false;
      if (intentionalClose) return;
      setStatus("Disconnected; retrying…", "error");
      const wait = Math.min(500 * 2 ** reconnects++, 10_000);
      window.setTimeout(() => {
        if (!intentionalClose && socket === null && capability.value.trim()) connect();
      }, wait);
    });
  };

  inviteButton.addEventListener("click", async () => {
    const secret = admin.value;
    if (!secret) {
      setStatus("Enter the world admin token to create an invite", "error");
      return;
    }
    inviteButton.disabled = true;
    try {
      const response = await fetch("/api/world/invite", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-world-admin-token": secret,
        },
        body: JSON.stringify({ display_name: name.value || "guest", world: selectedWorld() }),
      });
      if (!response.ok) {
        throw new Error(response.status === 401 ? "Admin token refused" : `Invite failed (${response.status})`);
      }
      const result = (await response.json()) as { capability: string; display_name: string };
      showCapability(result.capability);
      admin.value = "";
      setStatus("Invite created. Copy it or join this browser.", "good");
      appendLine(log, `Invite created for ${result.display_name}`);
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Invite failed", "error");
    } finally {
      inviteButton.disabled = false;
    }
  });

  installButton.addEventListener("click", () => {
    const secret = admin.value;
    const file = moduleInput.files?.[0];
    if (!secret) {
      setStatus("Enter the world admin token to install a module", "error");
      return;
    }
    if (!file) {
      setStatus("Choose a compiled .wasm module first", "error");
      return;
    }
    if (file.size === 0 || file.size > 16 * 1024 * 1024) {
      setStatus("Wasm module must be between 1 byte and 16 MiB", "error");
      return;
    }
    const seed = parseWorldSeed(moduleSeed.value);
    if (seed === null) {
      setStatus("Seed must be a non-negative safe integer", "error");
      return;
    }
    installButton.disabled = true;
    setStatus(`Uploading ${file.name} (${file.size} bytes)…`);
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const upload = new WebSocket(`${scheme}//${location.host}/ws/world`);
    let uploadStarted = false;
    upload.addEventListener("open", () => {
      upload.send(
        JSON.stringify({
          type: "install_begin",
          admin_token: secret,
          total_bytes: file.size,
          // The host computes the canonical Iroh BLAKE3 hash and returns it.
          module_hash: null,
          seed,
          world: selectedWorld(),
        }),
      );
    });
    upload.addEventListener("message", async (message) => {
      let frame: WorldFrame;
      try {
        frame = JSON.parse(String(message.data)) as typeof frame;
      } catch {
        setStatus("Host returned an invalid upload response", "error");
        upload.close();
        return;
      }
      if (frame.type === "error") {
        setStatus(frame.message ?? "Module install failed", "error");
        upload.close();
        return;
      }
      if (frame.type === "upload_ready" && !uploadStarted) {
        uploadStarted = true;
        const chunkBytes = frame.chunk_bytes;
        if (!chunkBytes || chunkBytes < 1 || chunkBytes > 6 * 1024) {
          setStatus("Host returned an invalid module chunk size", "error");
          upload.close();
          return;
        }
        try {
          for (let offset = 0; offset < file.size; offset += chunkBytes) {
            while (upload.bufferedAmount > 256 * 1024) {
              await new Promise((resolve) => setTimeout(resolve, 10));
            }
            if (upload.readyState !== WebSocket.OPEN) throw new Error("upload connection closed");
            const bytes = new Uint8Array(await file.slice(offset, offset + chunkBytes).arrayBuffer());
            const dataHex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
            upload.send(JSON.stringify({ type: "install_chunk", offset, data_hex: dataHex }));
          }
          upload.send(JSON.stringify({ type: "install_end" }));
          setStatus("Module received; host is validating and activating it…");
        } catch (error) {
          setStatus(error instanceof Error ? error.message : "Module upload failed", "error");
          upload.close();
        }
        return;
      }
      if (frame.type === "module_installed") {
        setStatus(`World program installed: ${frame.module_hash ?? "ok"}`, "good");
        appendLine(log, `program installed ${frame.module_hash ?? ""}`);
        admin.value = "";
        upload.close();
      }
    });
    upload.addEventListener("error", () => setStatus("Module upload connection failed", "error"));
    upload.addEventListener("close", () => {
      installButton.disabled = false;
    });
  });

  compileButton.addEventListener("click", async () => {
    const secret = admin.value;
    const selected = Array.from(sourceInput.files ?? []);
    if (!secret) {
      setStatus("Enter the world admin token to compile Roc sources", "error");
      return;
    }
    if (selected.length === 0) {
      setStatus("Choose a .roc source set with main.roc first", "error");
      return;
    }
    if (selected.length > 8) {
      setStatus("A compile request can contain at most 8 files", "error");
      return;
    }
    const ordered = [...selected].sort((a, b) =>
      a.name === "main.roc" ? -1 : b.name === "main.roc" ? 1 : a.name.localeCompare(b.name),
    );
    if (ordered[0]?.name !== "main.roc") {
      setStatus("Select main.roc; it must be the first compile file", "error");
      return;
    }
    const files: Array<{ name: string; content: string }> = [];
    let totalBytes = 0;
    const names = new Set<string>();
    for (const file of ordered) {
      if (!/^(?!\.)[A-Za-z0-9._-]{1,60}\.roc$/.test(file.name)) {
        setStatus(`${file.name} is not a safe .roc filename`, "error");
        return;
      }
      if (names.has(file.name)) {
        setStatus(`The source set contains more than one ${file.name}`, "error");
        return;
      }
      names.add(file.name);
      if (file.size > 128 * 1024) {
        setStatus(`${file.name} exceeds 128 KiB`, "error");
        return;
      }
      totalBytes += file.size;
      if (totalBytes > 256 * 1024) {
        setStatus("Roc sources exceed the 256 KiB total limit", "error");
        return;
      }
      files.push({ name: file.name, content: await file.text() });
    }
    const seed = parseWorldSeed(moduleSeed.value);
    if (seed === null) {
      setStatus("Seed must be a non-negative safe integer", "error");
      return;
    }
    compileButton.disabled = true;
    setStatus(`Reading ${files.length} Roc source file${files.length === 1 ? "" : "s"} (${totalBytes} bytes)…`);
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const compiler = new WebSocket(`${scheme}//${location.host}/ws/world`);
    compiler.addEventListener("open", () => {
      setStatus("Sources sent; compiling and installing on the host…");
      compiler.send(
        JSON.stringify({
          type: "compile",
          admin_token: secret,
          files,
          seed,
          native: false,
          world: selectedWorld(),
        }),
      );
    });
    compiler.addEventListener("message", (message) => {
      let frame: WorldFrame;
      try {
        frame = JSON.parse(String(message.data)) as WorldFrame;
      } catch {
        setStatus("Host returned an invalid compile response", "error");
        compiler.close();
        return;
      }
      if (frame.type === "error") {
        setStatus(frame.message ?? "Roc compilation failed", "error");
        appendLine(log, `compile error: ${frame.message ?? "Roc compilation failed"}`);
        compiler.close();
      } else if (frame.type === "compiled") {
        setStatus(`Roc program installed: ${frame.module_hash ?? "ok"}`, "good");
        appendLine(log, `Roc program installed ${frame.module_hash ?? ""}`);
        if (frame.diagnostics) appendLine(log, frame.diagnostics);
        admin.value = "";
        compiler.close();
      }
    });
    compiler.addEventListener("error", () => setStatus("Roc compile connection failed", "error"));
    compiler.addEventListener("close", () => {
      compileButton.disabled = false;
    });
  });

  joinButton.addEventListener("click", () => {
    hostMode = false;
    chatForm.hidden = false;
    inputForm.hidden = false;
    view.tabIndex = 0;
    if (socket) {
      // #1461: retire the old socket BEFORE closing it - clear the
      // current-socket reference first, so when its close event fires
      // (asynchronously, after this handler finishes) the identity
      // guard in connect() treats it as stale: no reconnect, no status
      // overwrite. The old code set `socket.onclose = null`, which was
      // a no-op (connect() registers with addEventListener, never
      // onclose), and reset intentionalClose in the same task, before
      // the async close event could ever observe it.
      intentionalClose = true;
      const retired = socket;
      socket = null;
      retired.close();
    }
    intentionalClose = false;
    connect();
  });

  chatForm.addEventListener("submit", (event) => {
    event.preventDefault();
    if (hostMode) {
      setStatus("Host view is read-only", "error");
      return;
    }
    const text = chatInput.value.trim();
    if (!text) return;
    if (socket?.readyState !== WebSocket.OPEN) {
      setStatus("Join the world before chatting", "error");
      return;
    }
    socket.send(JSON.stringify({ type: "chat", text }));
    chatInput.value = "";
  });

  inputForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const text = gameInput.value.trim();
    if (!text) return;
    if (!sendInputBytes(new TextEncoder().encode(text))) return;
    gameInput.value = "";
  });

  copyButton.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(capability.value);
      setStatus("Capability copied", "good");
    } catch {
      capability.select();
      document.execCommand("copy");
      setStatus("Capability selected for copying", "good");
    }
  });

  if (capability.value) connect();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initWorld, { once: true });
} else {
  initWorld();
}
