import * as PropertySymbol from '../PropertySymbol.js';
/**
 * Keyframe effect.
 *
 * @see https://drafts.csswg.org/web-animations-1/#the-keyframeeffect-interface
 */
export default class KeyframeEffect {
    target;
    pseudoElement = null;
    composite;
    iterationComposite;
    #keyframes;
    #timing;
    /**
     * Constructor.
     *
     * @param target Target element.
     * @param keyframes Keyframes.
     * @param options Timing options.
     */
    constructor(target, keyframes, options = {}) {
        this.target = target;
        this.#keyframes = this.#processKeyframes(keyframes);
        this.#timing = this.#processTiming(options);
        this.composite = typeof options === 'number' ? 'replace' : (options.composite ?? 'replace');
        this.iterationComposite =
            typeof options === 'number' ? 'replace' : (options.iterationComposite ?? 'replace');
    }
    /**
     * Returns the timing properties.
     *
     * @returns Timing properties.
     */
    getTiming() {
        return { ...this.#timing };
    }
    /**
     * Returns computed timing properties.
     *
     * @returns Computed timing properties.
     */
    getComputedTiming() {
        const duration = this.#timing.duration === 'auto' ? 0 : this.#timing.duration;
        const activeDuration = duration * this.#timing.iterations;
        return {
            ...this.#timing,
            activeDuration,
            currentIteration: null,
            endTime: Math.max(0, this.#timing.delay + activeDuration + this.#timing.endDelay),
            localTime: null,
            progress: null
        };
    }
    /**
     * Updates timing properties.
     *
     * @param timing Timing properties.
     */
    updateTiming(timing = {}) {
        this.#timing = this.#processTiming({ ...this.#timing, ...timing });
    }
    /**
     * Returns computed keyframes.
     *
     * @returns Keyframes.
     */
    getKeyframes() {
        return this.#keyframes.map((keyframe) => ({ ...keyframe }));
    }
    /**
     * Replaces the keyframes.
     *
     * @param keyframes Keyframes.
     */
    setKeyframes(keyframes) {
        this.#keyframes = this.#processKeyframes(keyframes);
    }
    /**
     *
     * @param options
     */
    #processTiming(options) {
        const timing = typeof options === 'number' ? { duration: options } : options;
        const duration = timing.duration ?? 'auto';
        const iterations = timing.iterations ?? 1;
        if ((duration !== 'auto' && (!Number.isFinite(duration) || duration < 0)) || iterations < 0) {
            throw new this[PropertySymbol.window].TypeError('Invalid animation timing options.');
        }
        return {
            delay: timing.delay ?? 0,
            direction: timing.direction ?? 'normal',
            duration,
            easing: timing.easing ?? 'linear',
            endDelay: timing.endDelay ?? 0,
            fill: timing.fill ?? 'auto',
            iterationStart: timing.iterationStart ?? 0,
            iterations
        };
    }
    /**
     *
     * @param keyframes
     */
    #processKeyframes(keyframes) {
        if (!keyframes) {
            return [];
        }
        if (Array.isArray(keyframes)) {
            const processed = keyframes.map((keyframe) => this.#processKeyframe(keyframe));
            this.#computeOffsets(processed);
            return processed;
        }
        const propertyNames = Object.keys(keyframes).filter((property) => !['composite', 'easing', 'offset'].includes(property));
        const length = Math.max(0, ...['composite', 'easing', 'offset'].map((property) => Array.isArray(keyframes[property]) ? keyframes[property].length : 0), ...propertyNames.map((property) => Array.isArray(keyframes[property]) ? keyframes[property].length : 1));
        const offsets = Array.isArray(keyframes.offset) ? keyframes.offset : [keyframes.offset ?? null];
        const easings = Array.isArray(keyframes.easing)
            ? keyframes.easing
            : [keyframes.easing ?? 'linear'];
        const composites = Array.isArray(keyframes.composite)
            ? keyframes.composite
            : [keyframes.composite ?? 'auto'];
        const processed = Array.from({ length }, (_, index) => ({
            offset: offsets[index] ?? null,
            easing: easings[index] ?? 'linear',
            composite: composites[index] ?? 'auto',
            computedOffset: 0
        }));
        for (const property of propertyNames) {
            const values = Array.isArray(keyframes[property])
                ? keyframes[property]
                : [keyframes[property]];
            for (let index = 0; index < values.length; index++) {
                if (values[index] !== undefined) {
                    processed[index][property] = String(values[index]);
                }
            }
        }
        this.#computeOffsets(processed);
        return processed;
    }
    /**
     * Processes a keyframe.
     *
     * @param keyframe Keyframe.
     * @returns Computed keyframe.
     */
    #processKeyframe(keyframe) {
        const processed = {
            offset: keyframe.offset ?? null,
            easing: keyframe.easing ?? 'linear',
            composite: keyframe.composite ?? 'auto',
            computedOffset: 0
        };
        for (const property of Object.keys(keyframe)) {
            if (!['composite', 'easing', 'offset'].includes(property) &&
                keyframe[property] !== undefined) {
                processed[property] = String(keyframe[property]);
            }
        }
        return processed;
    }
    /**
     *
     * @param keyframes
     */
    #computeOffsets(keyframes) {
        let previousOffset = -Infinity;
        for (const keyframe of keyframes) {
            if (keyframe.offset !== null && keyframe.offset !== undefined) {
                if (keyframe.offset < previousOffset || keyframe.offset < 0 || keyframe.offset > 1) {
                    throw new this[PropertySymbol.window].TypeError('Keyframe offsets must be in non-decreasing order between 0 and 1.');
                }
                previousOffset = keyframe.offset;
            }
        }
        if (!keyframes.length) {
            return;
        }
        const computedOffsets = keyframes.map((keyframe) => keyframe.offset);
        if (computedOffsets[0] === null) {
            computedOffsets[0] = keyframes.length === 1 ? 1 : 0;
        }
        if (computedOffsets.at(-1) === null) {
            computedOffsets[computedOffsets.length - 1] = 1;
        }
        let startIndex = 0;
        while (startIndex < computedOffsets.length - 1) {
            let endIndex = startIndex + 1;
            while (computedOffsets[endIndex] === null) {
                endIndex++;
            }
            const startOffset = computedOffsets[startIndex];
            const endOffset = computedOffsets[endIndex];
            for (let index = startIndex + 1; index < endIndex; index++) {
                computedOffsets[index] =
                    startOffset +
                        ((endOffset - startOffset) * (index - startIndex)) / (endIndex - startIndex);
            }
            startIndex = endIndex;
        }
        for (let index = 0; index < keyframes.length; index++) {
            keyframes[index].computedOffset = computedOffsets[index];
        }
    }
}
//# sourceMappingURL=KeyframeEffect.js.map