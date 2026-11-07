import * as PropertySymbol from '../PropertySymbol.js';
import EventTarget from '../event/EventTarget.js';
import DOMExceptionNameEnum from '../exception/DOMExceptionNameEnum.js';
/**
 * Animation.
 *
 * @see https://drafts.csswg.org/web-animations-1/#the-animation-interface
 */
export default class Animation extends EventTarget {
    id = '';
    effect;
    timeline;
    startTime = null;
    playbackRate = 1;
    replaceState = 'active';
    ready;
    finished;
    #playState = 'idle';
    #currentTime = null;
    #timer = null;
    #resolveFinished = null;
    #rejectFinished = null;
    [PropertySymbol.propertyEventListeners] = new Map();
    /**
     * Constructor.
     *
     * @param effect Animation effect.
     * @param timeline Animation timeline.
     */
    constructor(effect = null, timeline) {
        super();
        this.effect = effect;
        this.timeline =
            timeline === undefined ? this[PropertySymbol.window].document.timeline : timeline;
        this.ready = Promise.resolve(this);
        this.finished = this.#createFinishedPromise();
    }
    // Events
    /* eslint-disable jsdoc/require-jsdoc */
    get oncancel() {
        return this[PropertySymbol.propertyEventListeners].get('oncancel') ?? null;
    }
    set oncancel(value) {
        this[PropertySymbol.propertyEventListeners].set('oncancel', value);
    }
    get onfinish() {
        return this[PropertySymbol.propertyEventListeners].get('onfinish') ?? null;
    }
    set onfinish(value) {
        this[PropertySymbol.propertyEventListeners].set('onfinish', value);
    }
    get onremove() {
        return this[PropertySymbol.propertyEventListeners].get('onremove') ?? null;
    }
    set onremove(value) {
        this[PropertySymbol.propertyEventListeners].set('onremove', value);
    }
    /* eslint-enable jsdoc/require-jsdoc */
    /**
     * Returns the current time.
     *
     * @returns Current time.
     */
    get currentTime() {
        const timelineTime = this.timeline?.currentTime;
        if (this.#playState === 'running' && this.startTime !== null && timelineTime != null) {
            return (timelineTime - this.startTime) * this.playbackRate;
        }
        return this.#currentTime;
    }
    /**
     * Sets the current time.
     *
     * @param value Current time.
     */
    set currentTime(value) {
        this.#currentTime = value;
        const timelineTime = this.timeline?.currentTime;
        if (value !== null && timelineTime != null && this.playbackRate !== 0) {
            this.startTime = timelineTime - value / this.playbackRate;
        }
        if (this.#playState === 'running') {
            this.#scheduleFinish();
        }
    }
    /**
     * Returns the play state.
     *
     * @returns Play state.
     */
    get playState() {
        return this.#playState;
    }
    /**
     * Returns whether a play or pause task is pending.
     *
     * @returns Pending state.
     */
    get pending() {
        return false;
    }
    /**
     * Starts or resumes playback.
     */
    play() {
        if (this.#playState === 'finished' || this.#playState === 'idle') {
            this.finished = this.#createFinishedPromise();
        }
        const currentTime = this.#currentTime ?? (this.playbackRate < 0 ? this.#getEndTime() : 0);
        this.#currentTime = currentTime;
        const timelineTime = this.timeline?.currentTime;
        if (timelineTime != null && this.playbackRate !== 0) {
            this.startTime = timelineTime - currentTime / this.playbackRate;
        }
        this.#playState = 'running';
        this.#scheduleFinish();
    }
    /**
     * Pauses playback.
     */
    pause() {
        if (this.#playState === 'running') {
            this.#currentTime = this.currentTime;
            this.startTime = null;
            this.#playState = 'paused';
            this.#clearTimer();
        }
    }
    /**
     * Reverses playback.
     */
    reverse() {
        this.playbackRate = this.playbackRate === 0 ? -1 : -this.playbackRate;
        this.play();
    }
    /**
     * Finishes playback.
     */
    finish() {
        if (this.playbackRate === 0) {
            throw new this[PropertySymbol.window].DOMException('The animation playback rate is zero.', DOMExceptionNameEnum.invalidStateError);
        }
        this.#clearTimer();
        this.#currentTime = this.playbackRate < 0 ? 0 : this.#getEndTime();
        this.startTime = null;
        this.#playState = 'finished';
        this.#resolveFinished?.(this);
        this.dispatchEvent(new this[PropertySymbol.window].Event('finish'));
    }
    /**
     * Cancels playback.
     */
    cancel() {
        if (this.#playState === 'idle') {
            return;
        }
        this.#clearTimer();
        this.#playState = 'idle';
        this.#currentTime = null;
        this.startTime = null;
        this.#rejectFinished?.(new this[PropertySymbol.window].DOMException('The animation was canceled.', DOMExceptionNameEnum.abortError));
        this.dispatchEvent(new this[PropertySymbol.window].Event('cancel'));
    }
    /**
     * Persists the animation.
     */
    persist() {
        this.replaceState = 'persisted';
    }
    /**
     * Updates the playback rate.
     *
     * @param playbackRate Playback rate.
     */
    updatePlaybackRate(playbackRate) {
        const currentTime = this.currentTime;
        this.playbackRate = playbackRate;
        this.currentTime = currentTime;
    }
    /**
     * Gets the end time of the animation.
     *
     * @returns End time.
     */
    #getEndTime() {
        return this.effect?.getComputedTiming().endTime ?? 0;
    }
    /**
     * Creates a new finished promise.
     *
     * @returns Finished promise.
     */
    #createFinishedPromise() {
        return new Promise((resolve, reject) => {
            this.#resolveFinished = resolve;
            this.#rejectFinished = reject;
        });
    }
    /**
     * Schedules the finish of the animation.
     */
    #scheduleFinish() {
        this.#clearTimer();
        if (!this.effect || this.playbackRate === 0) {
            return;
        }
        const currentTime = this.currentTime ?? 0;
        const remaining = this.playbackRate < 0
            ? currentTime / -this.playbackRate
            : (this.#getEndTime() - currentTime) / this.playbackRate;
        if (Number.isFinite(remaining)) {
            this.#timer = this[PropertySymbol.window].setTimeout(() => this.finish(), Math.max(0, remaining));
        }
    }
    /**
     * Clears the finish timer.
     */
    #clearTimer() {
        if (this.#timer !== null) {
            this[PropertySymbol.window].clearTimeout(this.#timer);
            this.#timer = null;
        }
    }
}
//# sourceMappingURL=Animation.js.map