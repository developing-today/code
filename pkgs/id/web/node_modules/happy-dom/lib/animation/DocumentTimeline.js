import AnimationTimeline from './AnimationTimeline.js';
/**
 * Document timeline.
 *
 * @see https://drafts.csswg.org/web-animations-1/#the-documenttimeline-interface
 */
export default class DocumentTimeline extends AnimationTimeline {
    #originTime;
    /**
     * Constructor.
     *
     * @param options Options.
     */
    constructor(options = {}) {
        super();
        this.#originTime = options.originTime ?? 0;
    }
    /**
     * Returns the current timeline time.
     *
     * @returns Current time in milliseconds.
     */
    get currentTime() {
        return performance.now() - this.#originTime;
    }
}
//# sourceMappingURL=DocumentTimeline.js.map