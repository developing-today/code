/** Browser presentation for the server-authoritative world session. */

type WorldFrame = {
  type: "snapshot" | "event" | "view" | "records" | "invite" | "upload_ready" | "module_installed" | "error";
  snapshot?: {
    world_id: string;
    current_sequence: number;
    participants: Array<{ id: number; display_name: string }>;
  };
  event?: {
    sequence: number;
    participant_id: number;
    kind: { kind: "chat" | "input"; data: string | number[] };
  };
  sequence?: number;
  data?: string;
  invite?: { capability?: string; participant_id?: number; display_name?: string };
  capability?: string;
  records?: Record<string, unknown>;
  next_after?: string | null;
  chunk_bytes?: number;
  module_hash?: string;
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
  const moduleSeed = element<HTMLInputElement>(root, "[data-world-seed]");
  const installButton = element<HTMLButtonElement>(root, "[data-world-install]");
  const joinButton = element<HTMLButtonElement>(root, "[data-world-join]");
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
    !moduleSeed ||
    !installButton ||
    !joinButton ||
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
  let records: Record<string, unknown> = {};
  let recordsAfter: string | null = null;
  let recordsPending = false;
  let reconnects = 0;
  let intentionalClose = false;

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
        requestRecords();
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
        if (frame.data !== undefined) view.textContent = frame.data;
        break;
      case "records": {
        if (!recordsPending) return;
        Object.assign(records, frame.records ?? {});
        recordsAfter = frame.next_after ?? null;
        if (recordsAfter === null) {
          recordsPending = false;
          recordsView.textContent = JSON.stringify(records, null, 2);
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
    if (socket?.readyState !== WebSocket.OPEN) {
      setStatus("Join the world before reading records", "error");
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

  recordsRefresh.addEventListener("click", requestRecords);

  const connect = () => {
    const token = capability.value.trim();
    if (!token) {
      setStatus("Create or paste a guest capability first", "error");
      return;
    }
    intentionalClose = false;
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    socket = new WebSocket(`${scheme}//${location.host}/ws/world`);
    setStatus("Connecting…");
    socket.addEventListener("open", () => {
      reconnects = 0;
      socket?.send(JSON.stringify({ type: "join", capability: token, after: cursor || null, world: selectedWorld() }));
    });
    socket.addEventListener("message", (event) => receive(String(event.data)));
    socket.addEventListener("error", () => setStatus("World connection failed", "error"));
    socket.addEventListener("close", () => {
      if (intentionalClose) return;
      setStatus("Disconnected; retrying…", "error");
      const wait = Math.min(500 * 2 ** reconnects++, 10_000);
      window.setTimeout(() => {
        if (!intentionalClose && capability.value.trim()) connect();
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
    const seed = Number(moduleSeed.value);
    if (!Number.isSafeInteger(seed) || seed < 0) {
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

  joinButton.addEventListener("click", () => {
    if (socket) {
      intentionalClose = true;
      socket.onclose = null;
      socket.close();
      socket = null;
    }
    intentionalClose = false;
    connect();
  });

  chatForm.addEventListener("submit", (event) => {
    event.preventDefault();
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
    if (socket?.readyState !== WebSocket.OPEN) {
      setStatus("Join the world before sending game input", "error");
      return;
    }
    const dataHex = Array.from(new TextEncoder().encode(text), (byte) => byte.toString(16).padStart(2, "0")).join("");
    socket.send(JSON.stringify({ type: "input", data_hex: dataHex }));
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
