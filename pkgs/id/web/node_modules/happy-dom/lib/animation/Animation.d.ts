import * as PropertySymbol from '../PropertySymbol.js';
import EventTarget from '../event/EventTarget.js';
import type Event from '../event/Event.js';
import type AnimationTimeline from './AnimationTimeline.js';
import type KeyframeEffect from './KeyframeEffect.js';
import type BrowserWindow from '../window/BrowserWindow.js';
export type TAnimationPlayState = 'finished' | 'idle' | 'paused' | 'running';
export type TAnimationReplaceState = 'active' | 'persisted' | 'removed';
/**
 * Animation.
 *
 * @see https://drafts.csswg.org/web-animations-1/#the-animation-interface
 */
export default class Animation extends EventTarget {
    #private;
    protected [PropertySymbol.window]: BrowserWindow;
    id: string;
    effect: KeyframeEffect | null;
    timeline: AnimationTimeline | null;
    startTime: number | null;
    playbackRate: number;
    replaceState: TAnimationReplaceState;
    ready: Promise<this>;
    finished: Promise<this>;
    [PropertySymbol.propertyEventListeners]: Map<string, ((event: Event) => void) | null>;
    /**
     * Constructor.
     *
     * @param effect Animation effect.
     * @param timeline Animation timeline.
     */
    constructor(effect?: KeyframeEffect | null, timeline?: AnimationTimeline | null);
    get oncancel(): ((event: Event) => void) | null;
    set oncancel(value: ((event: Event) => void) | null);
    get onfinish(): ((event: Event) => void) | null;
    set onfinish(value: ((event: Event) => void) | null);
    get onremove(): ((event: Event) => void) | null;
    set onremove(value: ((event: Event) => void) | null);
    /**
     * Returns the current time.
     *
     * @returns Current time.
     */
    get currentTime(): number | null;
    /**
     * Sets the current time.
     *
     * @param value Current time.
     */
    set currentTime(value: number | null);
    /**
     * Returns the play state.
     *
     * @returns Play state.
     */
    get playState(): TAnimationPlayState;
    /**
     * Returns whether a play or pause task is pending.
     *
     * @returns Pending state.
     */
    get pending(): boolean;
    /**
     * Starts or resumes playback.
     */
    play(): void;
    /**
     * Pauses playback.
     */
    pause(): void;
    /**
     * Reverses playback.
     */
    reverse(): void;
    /**
     * Finishes playback.
     */
    finish(): void;
    /**
     * Cancels playback.
     */
    cancel(): void;
    /**
     * Persists the animation.
     */
    persist(): void;
    /**
     * Updates the playback rate.
     *
     * @param playbackRate Playback rate.
     */
    updatePlaybackRate(playbackRate: number): void;
}
//# sourceMappingURL=Animation.d.ts.map