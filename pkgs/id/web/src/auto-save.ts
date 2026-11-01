/**
 * Auto-save manager for the id web editor.
 *
 * Serializes saves: at most one saveFn() call is ever in flight. A save
 * requested while one is in flight (or while a rate-limit retry is pending)
 * is coalesced into exactly one follow-up attempt, and a save completion
 * only marks the document "saved" when no edit arrived after that save
 * started (tracked with a monotonic edit generation).
 */

/** Result from saveFile() for AutoSaveManager to process */
export type SaveResult = { ok: true } | { ok: false; retryAfterMs?: number };

export const AUTOSAVE_DEBOUNCE_MS = 2000;

export type SaveState = "idle" | "unsaved" | "saving" | "saved" | "rate-limited" | "error";

export class AutoSaveManager {
  state: SaveState = "idle";
  private debounceTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private savedResetTimer: ReturnType<typeof setTimeout> | null = null;
  private saveFn: () => Promise<SaveResult>;
  /** Bumped on every local edit; a save covers edits up to the generation it started at. */
  private editGeneration = 0;
  /** Serialization latch: true while a saveFn() call is unresolved. */
  private saveInFlight = false;
  /** A save was requested while one was in flight or a retry was pending; run one follow-up. */
  private followUpRequested = false;
  /** The editor owning this manager closed; late completions must be ignored. */
  private disposed = false;

  constructor(saveFn: () => Promise<SaveResult>) {
    this.saveFn = saveFn;
  }

  /** Called when user makes a local edit (editor:change event) */
  onContentChange(): void {
    if (this.disposed) return;

    this.editGeneration++;

    // Clear any pending timers
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.savedResetTimer !== null) {
      clearTimeout(this.savedResetTimer);
      this.savedResetTimer = null;
    }

    this.state = "unsaved";
    this.updateIndicator();

    // Start debounce — save after 2s of no edits
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = null;
      this.triggerSave();
    }, AUTOSAVE_DEBOUNCE_MS);
  }

  /** Called when another client saves (NewVersion received) */
  onNewVersion(): void {
    if (this.disposed) return;

    // Cancel any pending save — their version is newer
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    if (this.savedResetTimer !== null) {
      clearTimeout(this.savedResetTimer);
      this.savedResetTimer = null;
    }
    this.followUpRequested = false;

    this.state = "saved";
    this.updateIndicator();

    // Reset indicator after 2s
    this.savedResetTimer = setTimeout(() => {
      this.savedResetTimer = null;
      if (this.state === "saved") {
        this.state = "idle";
        this.updateIndicator();
      }
    }, 2000);
  }

  /** Manual save — Ctrl+S or button click. Cancels debounce and saves immediately. */
  saveNow(): void {
    if (this.disposed) return;

    // Cancel debounce timer — we're saving right now
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    // Don't save if already saving (button is disabled anyway)
    if (this.state === "saving" || this.state === "rate-limited") {
      return;
    }
    this.triggerSave();
  }

  /** Clean up all timers (called when editor closes) */
  cancel(): void {
    if (this.disposed) return;

    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    if (this.savedResetTimer !== null) {
      clearTimeout(this.savedResetTimer);
      this.savedResetTimer = null;
    }
    this.followUpRequested = false;
    this.state = "idle";
    this.updateIndicator();
    this.disposed = true;
  }

  /** Execute the save and handle the result, serialized against other saves */
  private async triggerSave(): Promise<void> {
    if (this.disposed) return;

    if (this.saveInFlight) {
      // Coalesce into exactly one follow-up once the in-flight save settles.
      this.followUpRequested = true;
      return;
    }
    if (this.retryTimer !== null) {
      // A rate-limit retry is already scheduled and will capture the current
      // document when it fires — coalesce into it instead of saving early
      // (which would defeat the backoff) or cancelling it outright.
      this.followUpRequested = true;
      return;
    }
    // This save captures the current document, so a pending debounce is moot.
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }

    this.saveInFlight = true;
    this.followUpRequested = false;
    const startGeneration = this.editGeneration;
    this.state = "saving";
    this.updateIndicator();

    let result: SaveResult;
    try {
      result = await this.saveFn();
    } catch {
      // A throwing saveFn is a generic save error; never wedge the latch.
      result = { ok: false };
    }
    this.saveInFlight = false;
    this.onSaveResult(result, startGeneration);
  }

  /** Transition state based on save outcome */
  private onSaveResult(result: SaveResult, startGeneration: number): void {
    if (this.disposed) return;

    if (result.ok) {
      if (this.followUpRequested || this.editGeneration !== startGeneration) {
        // Edits arrived after this save started (or a save was requested
        // during it): this completion does NOT cover them, so the document
        // must not read "saved". Run exactly one follow-up save.
        void this.triggerSave();
        return;
      }
      this.state = "saved";
      this.updateIndicator();

      // Reset to idle after 2s
      this.savedResetTimer = setTimeout(() => {
        this.savedResetTimer = null;
        if (this.state === "saved") {
          this.state = "idle";
          this.updateIndicator();
        }
      }, 2000);
    } else if (result.retryAfterMs) {
      // Rate limited — schedule retry. The retry captures the document when
      // it fires, so it also serves as the follow-up for any edits that
      // arrived during the failed save: consume the follow-up request here.
      this.followUpRequested = false;
      this.state = "rate-limited";
      this.updateIndicator();

      this.retryTimer = setTimeout(() => {
        this.retryTimer = null;
        this.triggerSave();
      }, result.retryAfterMs);
    } else {
      // Generic error — don't auto-retry a save that covered everything
      // (prevents infinite loops on network outage). But an edit coalesced
      // during this save is not covered by it: consume the request and run
      // exactly one follow-up (#1616). The follow-up starts with the request
      // cleared, so if it also fails generically with no new edits, it stops
      // in the error state below — no retry loop.
      const followUp = this.followUpRequested || this.editGeneration !== startGeneration;
      this.followUpRequested = false;
      this.state = "error";
      this.updateIndicator();

      // Show error for 2s, then revert to "save •" (content is still unsaved)
      this.savedResetTimer = setTimeout(() => {
        this.savedResetTimer = null;
        if (this.state === "error") {
          this.state = "unsaved";
          this.updateIndicator();
        }
      }, 2000);

      if (followUp) {
        void this.triggerSave();
      }
    }
  }

  /** Update the save button text/state to reflect current state */
  updateIndicator(): void {
    if (this.disposed) return;

    const saveBtn = document.getElementById("save-btn") as HTMLButtonElement | null;
    if (!saveBtn) return;

    switch (this.state) {
      case "idle":
        saveBtn.textContent = "save";
        saveBtn.disabled = false;
        break;
      case "unsaved":
        saveBtn.textContent = "save •";
        saveBtn.disabled = false;
        break;
      case "saving":
        saveBtn.textContent = "saving…";
        saveBtn.disabled = true;
        break;
      case "saved":
        saveBtn.textContent = "saved ✓";
        saveBtn.disabled = false;
        break;
      case "rate-limited":
        saveBtn.textContent = "retry…";
        saveBtn.disabled = true;
        break;
      case "error":
        saveBtn.textContent = "error!";
        saveBtn.disabled = false;
        break;
    }
  }
}
