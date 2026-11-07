import DOMExceptionNameEnum from '../../exception/DOMExceptionNameEnum.js';
import CSSPropertyManager from './property-manager/CSSPropertyManager.js';
import * as PropertySymbol from '../../PropertySymbol.js';
import CSSComputedStyle from './computed-style/CSSComputedStyle.js';
import ClassMethodBinder from '../../utilities/ClassMethodBinder.js';
import CSSPropertyList from './CSSPropertyList.js';
/* eslint-enable @typescript-eslint/no-empty-object-type */
/* eslint-enable @typescript-eslint/naming-convention */
/**
 * CSS Style Declaration.
 *
 * @see https://developer.mozilla.org/en-US/docs/Web/API/CSSStyleDeclaration
 */
export default class CSSStyleDeclaration {
    // Public properties
    parentRule = null;
    // Internal properties
    [PropertySymbol.window];
    [PropertySymbol.element];
    [PropertySymbol.computed];
    [PropertySymbol.cache] = {
        attributeValue: null,
        propertyManager: null
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
    constructor(illegalConstructorSymbol, window, options) {
        if (illegalConstructorSymbol !== PropertySymbol.illegalConstructor) {
            throw new TypeError('Illegal constructor');
        }
        this[PropertySymbol.window] = window;
        this[PropertySymbol.element] = options?.element || null;
        this[PropertySymbol.computed] = options?.element ? !!options?.computed : false;
        const methodBinder = new ClassMethodBinder(this, [CSSStyleDeclaration]);
        return new Proxy(this, {
            get: (target, property) => {
                if (property in target || typeof property === 'symbol') {
                    methodBinder.bind(property);
                    return target[property];
                }
                if (CSSPropertyList.kebabCase[property]) {
                    return target.getPropertyValue(property);
                }
                if (CSSPropertyList.camelCase[property]) {
                    return target.getPropertyValue(CSSPropertyList.camelCase[property]);
                }
                const index = Number(property);
                if (!isNaN(index)) {
                    return target.item(index) || undefined;
                }
            },
            set(target, property, newValue) {
                methodBinder.bind(property);
                if (property in target || typeof property === 'symbol') {
                    target[property] = newValue;
                    return true;
                }
                if (CSSPropertyList.kebabCase[property]) {
                    target.setProperty(property, newValue);
                    return true;
                }
                if (CSSPropertyList.camelCase[property]) {
                    target.setProperty(CSSPropertyList.camelCase[property], newValue);
                    return true;
                }
                return true;
            },
            deleteProperty(target, property) {
                if (property in target || typeof property === 'symbol') {
                    delete target[property];
                    return true;
                }
                return true;
            },
            ownKeys(target) {
                const indexKeys = [];
                for (let i = 0, max = target.length; i < max; i++) {
                    indexKeys.push(String(i));
                }
                return indexKeys.concat(Object.keys(CSSPropertyList.camelCase));
            },
            has(target, property) {
                if (property in target || typeof property === 'symbol') {
                    return true;
                }
                if (CSSPropertyList.kebabCase[property] ||
                    CSSPropertyList.camelCase[property]) {
                    return true;
                }
                const index = Number(property);
                if (!isNaN(index)) {
                    return true;
                }
                return false;
            },
            defineProperty(target, property, descriptor) {
                methodBinder.preventBinding(property);
                if (property in target) {
                    Object.defineProperty(target, property, descriptor);
                    return true;
                }
                return false;
            },
            getOwnPropertyDescriptor(target, property) {
                if (property in target || typeof property === 'symbol') {
                    return;
                }
                if (CSSPropertyList.kebabCase[property] ||
                    CSSPropertyList.camelCase[property]) {
                    return {
                        value: target.getPropertyValue(property),
                        writable: true,
                        enumerable: true,
                        configurable: true
                    };
                }
                const index = Number(property);
                if (!isNaN(index)) {
                    const propertyName = target.item(index);
                    if (propertyName) {
                        return {
                            value: propertyName,
                            writable: true,
                            enumerable: true,
                            configurable: true
                        };
                    }
                }
            }
        });
    }
    /**
     * Returns length.
     *
     * @returns Length.
     */
    get length() {
        return this[PropertySymbol.getPropertyManager]().size();
    }
    /**
     * Returns the style declaration as a CSS text.
     *
     * @returns CSS text.
     */
    get cssText() {
        if (this[PropertySymbol.element] && this[PropertySymbol.computed]) {
            return '';
        }
        return this[PropertySymbol.getPropertyManager]().toString();
    }
    /**
     * Sets CSS text.
     *
     * @param cssText CSS text.
     */
    set cssText(cssText) {
        if (this[PropertySymbol.computed]) {
            throw new this[PropertySymbol.window].DOMException(`Failed to execute 'cssText' on 'CSSStyleDeclaration': These styles are computed, and the properties are therefore read-only.`, DOMExceptionNameEnum.domException);
        }
        if (this[PropertySymbol.element]) {
            this[PropertySymbol.cache].propertyManager = new CSSPropertyManager({ cssText });
            this[PropertySymbol.cache].attributeValue = cssText;
            this[PropertySymbol.element].setAttribute('style', this[PropertySymbol.cache].propertyManager.toString());
        }
        else {
            this[PropertySymbol.cache].propertyManager = new CSSPropertyManager({ cssText });
        }
    }
    /**
     * Returns item.
     *
     * @param index Index.
     * @returns Item.
     */
    item(index) {
        return this[PropertySymbol.getPropertyManager]().item(index);
    }
    /**
     * Set a property.
     *
     * @param name Property name.
     * @param value Value. Must not contain "!important" as that should be set using the priority parameter.
     * @param [priority] Can be "important", or an empty string.
     */
    setProperty(name, value, priority) {
        if (this[PropertySymbol.computed]) {
            throw new this[PropertySymbol.window].DOMException(`Failed to execute 'setProperty' on 'CSSStyleDeclaration': These styles are computed, and therefore the '${name}' property is read-only.`, DOMExceptionNameEnum.domException);
        }
        if (priority !== '' && priority !== undefined && priority !== 'important') {
            return;
        }
        const stringValue = String(value).trim();
        const propertyManager = this[PropertySymbol.getPropertyManager]();
        if (stringValue) {
            propertyManager.set(name, stringValue, !!priority);
        }
        else {
            propertyManager.remove(name);
        }
        if (this[PropertySymbol.element]) {
            this[PropertySymbol.cache].attributeValue = propertyManager.toString();
            if (this[PropertySymbol.cache].attributeValue) {
                this[PropertySymbol.element].setAttribute('style', this[PropertySymbol.cache].attributeValue);
            }
            else {
                this[PropertySymbol.element].removeAttribute('style');
            }
        }
    }
    /**
     * Removes a property.
     *
     * @param name Property name in kebab case.
     * @param value Value. Must not contain "!important" as that should be set using the priority parameter.
     * @param [priority] Can be "important", or an empty string.
     */
    removeProperty(name) {
        if (this[PropertySymbol.computed]) {
            throw new this[PropertySymbol.window].DOMException(`Failed to execute 'removeProperty' on 'CSSStyleDeclaration': These styles are computed, and therefore the '${name}' property is read-only.`, DOMExceptionNameEnum.domException);
        }
        const propertyManager = this[PropertySymbol.getPropertyManager]();
        propertyManager.remove(name);
        if (this[PropertySymbol.element]) {
            this[PropertySymbol.cache].attributeValue = propertyManager.toString();
            if (this[PropertySymbol.cache].attributeValue) {
                this[PropertySymbol.element].setAttribute('style', this[PropertySymbol.cache].attributeValue);
            }
            else {
                this[PropertySymbol.element].removeAttribute('style');
            }
        }
    }
    /**
     * Returns a property.
     *
     * @param name Property name in kebab case.
     * @returns Property value.
     */
    getPropertyValue(name) {
        return this[PropertySymbol.getPropertyManager]().get(name)?.value || '';
    }
    /**
     * Returns a property.
     *
     * @param name Property name in kebab case.
     * @returns "important" if set to be important.
     */
    getPropertyPriority(name) {
        return this[PropertySymbol.getPropertyManager]().get(name)?.important ? 'important' : '';
    }
    /**
     * Returns property manager.
     *
     * @returns Property manager.
     */
    [PropertySymbol.getPropertyManager]() {
        const element = this[PropertySymbol.element];
        const cache = this[PropertySymbol.cache];
        if (!element) {
            if (!cache.propertyManager) {
                cache.propertyManager = new CSSPropertyManager();
            }
            return cache.propertyManager;
        }
        if (this[PropertySymbol.computed]) {
            return new CSSComputedStyle(element).getComputedStyle();
        }
        const attributeValue = element.getAttribute('style') || '';
        if (cache.attributeValue !== attributeValue) {
            cache.propertyManager = new CSSPropertyManager({ cssText: attributeValue });
        }
        return cache.propertyManager;
    }
}
//# sourceMappingURL=CSSStyleDeclaration.js.map