import type Element from '../nodes/element/Element.js';
export type TCompositeOperation = 'accumulate' | 'add' | 'replace';
export type TKeyframeCompositeOperation = 'accumulate' | 'add' | 'auto' | 'replace';
export type TIterationCompositeOperation = 'accumulate' | 'replace';
export type TPlaybackDirection = 'alternate' | 'alternate-reverse' | 'normal' | 'reverse';
export type TFillMode = 'auto' | 'backwards' | 'both' | 'forwards' | 'none';
export type TKeyframeValue = string | number | null;
import * as PropertySymbol from '../PropertySymbol.js';
import type BrowserWindow from '../window/BrowserWindow.js';
export interface IKeyframe {
    [property: string]: TKeyframeValue | undefined;
    composite?: TKeyframeCompositeOperation;
    easing?: string;
    offset?: number | null;
}
export interface IPropertyIndexedKeyframes {
    [property: string]: TKeyframeValue | TKeyframeValue[] | undefined;
    composite?: TKeyframeCompositeOperation | TKeyframeCompositeOperation[];
    easing?: string | string[];
    offset?: number | null | Array<number | null>;
}
export interface IComputedKeyframe {
    [property: string]: string | number | null;
    composite: TKeyframeCompositeOperation;
    computedOffset: number;
    easing: string;
    offset: number | null;
}
export interface IEffectTiming {
    delay?: number;
    direction?: TPlaybackDirection;
    duration?: number | 'auto';
    easing?: string;
    endDelay?: number;
    fill?: TFillMode;
    iterationStart?: number;
    iterations?: number;
}
export interface IKeyframeEffectOptions extends IEffectTiming {
    composite?: TCompositeOperation;
    iterationComposite?: TIterationCompositeOperation;
}
export interface IComputedEffectTiming extends Required<IEffectTiming> {
    activeDuration: number;
    currentIteration: number | null;
    endTime: number;
    localTime: number | null;
    progress: number | null;
}
/**
 * Keyframe effect.
 *
 * @see https://drafts.csswg.org/web-animations-1/#the-keyframeeffect-interface
 */
export default class KeyframeEffect {
    #private;
    protected [PropertySymbol.window]: BrowserWindow;
    target: Element | null;
    pseudoElement: string | null;
    composite: TCompositeOperation;
    iterationComposite: TIterationCompositeOperation;
    /**
     * Constructor.
     *
     * @param target Target element.
     * @param keyframes Keyframes.
     * @param options Timing options.
     */
    constructor(target: Element | null, keyframes: IKeyframe[] | IPropertyIndexedKeyframes | null, options?: number | IKeyframeEffectOptions);
    /**
     * Returns the timing properties.
     *
     * @returns Timing properties.
     */
    getTiming(): Required<IEffectTiming>;
    /**
     * Returns computed timing properties.
     *
     * @returns Computed timing properties.
     */
    getComputedTiming(): IComputedEffectTiming;
    /**
     * Updates timing properties.
     *
     * @param timing Timing properties.
     */
    updateTiming(timing?: IEffectTiming): void;
    /**
     * Returns computed keyframes.
     *
     * @returns Keyframes.
     */
    getKeyframes(): IComputedKeyframe[];
    /**
     * Replaces the keyframes.
     *
     * @param keyframes Keyframes.
     */
    setKeyframes(keyframes: IKeyframe[] | IPropertyIndexedKeyframes | null): void;
}
//# sourceMappingURL=KeyframeEffect.d.ts.map