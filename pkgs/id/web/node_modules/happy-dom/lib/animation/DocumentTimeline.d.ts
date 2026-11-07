import AnimationTimeline from './AnimationTimeline.js';
export interface IDocumentTimelineOptions {
    originTime?: number;
}
/**
 * Document timeline.
 *
 * @see https://drafts.csswg.org/web-animations-1/#the-documenttimeline-interface
 */
export default class DocumentTimeline extends AnimationTimeline {
    #private;
    /**
     * Constructor.
     *
     * @param options Options.
     */
    constructor(options?: IDocumentTimelineOptions);
    /**
     * Returns the current timeline time.
     *
     * @returns Current time in milliseconds.
     */
    get currentTime(): number;
}
//# sourceMappingURL=DocumentTimeline.d.ts.map