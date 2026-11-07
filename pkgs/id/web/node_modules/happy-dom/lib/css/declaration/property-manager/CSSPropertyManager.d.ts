import type ICSSPropertyValue from './types/ICSSPropertyValue.js';
import type ICSSPropertyMap from './types/ICSSPropertyMap.js';
/**
 * CSS property manager.
 */
export default class CSSPropertyManager {
    properties: ICSSPropertyMap;
    private definedPropertyNames;
    /**
     * Constructor.
     *
     * @param [options] Options.
     * @param [options.cssText] CSS string.
     */
    constructor(options?: {
        cssText?: string;
    });
    /**
     * Returns property value.
     *
     * @param name Property name.
     * @returns Property value.
     */
    get(name: string): ICSSPropertyValue | null;
    /**
     * Removes a property.
     *
     * @param name Property name.
     */
    remove(name: string): void;
    /**
     * Sets a property
     *
     * @param name Name.
     * @param value Value.
     * @param important Important.
     * @returns Array of property names that were set.
     */
    set(name: string, value: string, important: boolean): string[];
    /**
     * Returns a clone.
     *
     * @returns Clone.
     */
    clone(): CSSPropertyManager;
    /**
     * Returns all CSS variables defined in this property manager.
     */
    getVariables(): {
        [k: string]: string;
    };
    /**
     * Returns size.
     *
     * @returns Size.
     */
    size(): number;
    /**
     * Returns property name.
     *
     * @param index Index.
     * @returns Property name.
     */
    item(index: number): string;
    /**
     * Converts properties to string.
     *
     * @returns String.
     */
    toString(): string;
}
//# sourceMappingURL=CSSPropertyManager.d.ts.map