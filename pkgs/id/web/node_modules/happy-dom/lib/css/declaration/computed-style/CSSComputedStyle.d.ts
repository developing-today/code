import type Element from '../../../nodes/element/Element.js';
import CSSPropertyManager from '../property-manager/CSSPropertyManager.js';
/**
 * CSS Style Declaration utility
 */
export default class CSSComputedStyle {
    private element;
    /**
     * Constructor.
     *
     * @param element Element.
     * @param [computed] Computed.
     */
    constructor(element: Element);
    /**
     * Returns style sheets.
     *
     * @param element Element.
     * @returns Style sheets.
     */
    getComputedStyle(): CSSPropertyManager;
    /**
     * Returns style sheets.
     *
     * @param root Root element.
     * @returns Style sheets.
     */
    private getStyleSheets;
    /**
     * Applies CSS text to elements.
     *
     * @param options Options.
     * @param options.elements Elements.
     * @param options.cssRules CSS rules.
     * @param [options.scopeElements] Scope elements.
     * @param [options.scopeElement] Scope element.
     * @param [options.hostElement] Host element.
     */
    private parseCSSRules;
    /**
     * Parses measurements in a value.
     *
     * @param options Options.
     * @param options.value Value.
     * @param options.rootFontSize Root font size.
     * @param options.parentFontSize Parent font size.
     * @param [options.parentSize] Parent width.
     * @returns CSS value.
     */
    private parseMeasurementsInValue;
    /**
     * Gets the cached property manager for a node.
     *
     * @param node Node.
     * @returns CSS property manager or null if not available.
     */
    private getCachedPropertyManager;
}
//# sourceMappingURL=CSSComputedStyle.d.ts.map