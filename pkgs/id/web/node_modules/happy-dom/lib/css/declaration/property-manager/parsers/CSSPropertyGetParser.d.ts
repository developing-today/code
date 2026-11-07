import type ICSSPropertyValue from '../types/ICSSPropertyValue.js';
import type ICSSPropertyMap from '../types/ICSSPropertyMap.js';
/**
 * Computed style property parser.
 */
export default class CSSPropertyGetParser {
    /**
     * Returns margin.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getMargin(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns padding.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getPadding(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns outline.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getOutline(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorder(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderTop(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderRight(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderBottom(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderLeft(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderColor(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderWidth(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderStyle(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border radius.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderRadius(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns border image.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBorderImage(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns background.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBackground(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns background position.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getBackgroundPosition(properties: ICSSPropertyMap): ICSSPropertyValue | null;
    /**
     * Returns flex.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getFlex(properties: {
        [k: string]: ICSSPropertyValue;
    }): ICSSPropertyValue | null;
    /**
     * Returns flex.
     *
     * @param properties Properties.
     * @returns Property value
     */
    static getFont(properties: {
        [k: string]: ICSSPropertyValue;
    }): ICSSPropertyValue | null;
    /**
     * Returns border.
     *
     * @param properties Properties.
     * @param position
     * @returns Property value
     */
    private static getBorderTopRightBottomLeft;
    /**
     * Returns a padding like property.
     *
     * @param properties Properties.
     * @param position
     * @param propertyNames
     * @returns Property value
     */
    private static getPaddingLikeProperty;
}
//# sourceMappingURL=CSSPropertyGetParser.d.ts.map