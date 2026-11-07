/**
 * CSS parser.
 */
export default class CSSTextParser {
    /**
     * Class construtor.
     *
     * @param cssText CSS string.
     * @param callback Callback.
     */
    static parse(cssText: string): {
        rules: Array<{
            name: string;
            value: string;
            important: boolean;
        }>;
        variables: {
            [name: string]: string;
        };
    };
}
//# sourceMappingURL=CSSTextParser.d.ts.map