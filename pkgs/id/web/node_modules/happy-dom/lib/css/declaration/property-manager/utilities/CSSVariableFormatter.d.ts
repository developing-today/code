/**
 * CSS variable formatter.
 */
export default class CSSVariableFormatter {
    /**
     * Formats CSS variable.
     *
     * @param value Value containing CSS variable.
     * @returns Parsed value.
     */
    static getVariable(value: string): string | null;
    /**
     * Resolves CSS variables into it's value.
     *
     * @param value Value containing CSS variable.
     * @param cssVariables CSS variables.
     * @returns Parsed value.
     */
    static resolveVariables(value: string, cssVariables: {
        [k: string]: string;
    }): string;
}
//# sourceMappingURL=CSSVariableFormatter.d.ts.map