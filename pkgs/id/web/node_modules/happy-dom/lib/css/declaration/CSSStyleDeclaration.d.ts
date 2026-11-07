import type Element from '../../nodes/element/Element.js';
import type CSSRule from '../CSSRule.js';
import CSSPropertyManager from './property-manager/CSSPropertyManager.js';
import * as PropertySymbol from '../../PropertySymbol.js';
import type BrowserWindow from '../../window/BrowserWindow.js';
import type ICSSStyleDeclaration from './ICSSStyleDeclaration.js';
export default interface CSSStyleDeclaration extends ICSSStyleDeclaration {
}
/**
 * CSS Style Declaration.
 *
 * @see https://developer.mozilla.org/en-US/docs/Web/API/CSSStyleDeclaration
 */
export default class CSSStyleDeclaration {
    [index: number]: string | undefined;
    readonly parentRule: CSSRule | null;
    [PropertySymbol.window]: BrowserWindow;
    [PropertySymbol.element]: Element | null;
    [PropertySymbol.computed]: boolean;
    [PropertySymbol.cache]: {
        attributeValue: string | null;
        propertyManager: CSSPropertyManager | null;
    };
    /**
     * Constructor.
     *
     * @param illegalConstructorSymbol Illegal constructor symbol.
     * @param window Window.
     * @param [options] Options.
     * @param [options.element] Element.
     * @param [options.computed] Computed.
     */
    constructor(illegalConstructorSymbol: symbol, window: BrowserWindow, options?: {
        element?: Element;
        computed?: boolean;
    });
    /**
     * Returns length.
     *
     * @returns Length.
     */
    get length(): number;
    /**
     * Returns the style declaration as a CSS text.
     *
     * @returns CSS text.
     */
    get cssText(): string;
    /**
     * Sets CSS text.
     *
     * @param cssText CSS text.
     */
    set cssText(cssText: string);
    /**
     * Returns item.
     *
     * @param index Index.
     * @returns Item.
     */
    item(index: number): string;
    /**
     * Set a property.
     *
     * @param name Property name.
     * @param value Value. Must not contain "!important" as that should be set using the priority parameter.
     * @param [priority] Can be "important", or an empty string.
     */
    setProperty(name: string, value: string, priority?: 'important' | '' | undefined): void;
    /**
     * Removes a property.
     *
     * @param name Property name in kebab case.
     * @param value Value. Must not contain "!important" as that should be set using the priority parameter.
     * @param [priority] Can be "important", or an empty string.
     */
    removeProperty(name: string): void;
    /**
     * Returns a property.
     *
     * @param name Property name in kebab case.
     * @returns Property value.
     */
    getPropertyValue(name: string): string;
    /**
     * Returns a property.
     *
     * @param name Property name in kebab case.
     * @returns "important" if set to be important.
     */
    getPropertyPriority(name: string): string;
    /**
     * Returns property manager.
     *
     * @returns Property manager.
     */
    private [PropertySymbol.getPropertyManager];
}
//# sourceMappingURL=CSSStyleDeclaration.d.ts.map