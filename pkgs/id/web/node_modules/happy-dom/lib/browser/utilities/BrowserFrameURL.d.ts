import type IBrowserFrame from '../types/IBrowserFrame.js';
import { URL } from 'url';
/**
 * Browser frame URL utility.
 */
export default class BrowserFrameURL {
    /**
     * Returns whether a target URL has the same origin as the origin URL.
     *
     * @param originURL Origin URL.
     * @param targetURL Target URL.
     * @returns Whether the URLs have the same origin.
     */
    static isSameOrigin(originURL: Pick<URL, 'origin'>, targetURL: URL): boolean;
    /**
     * Returns whether a target URL has an opaque origin instead of an inherited origin.
     *
     * @param targetURL Target URL.
     * @returns Whether the URL has an opaque origin.
     */
    static hasOpaqueOrigin(targetURL: URL): boolean;
    /**
     * Returns relative URL.
     *
     * @param frame Frame.
     * @param url URL.
     * @returns Relative URL.
     */
    static getRelativeURL(frame: IBrowserFrame, url?: string | URL | null): URL;
    /**
     * Returns whether a target URL has an inherited origin.
     *
     * @param targetURL Target URL.
     * @returns Whether the URL has an inherited origin.
     */
    private static hasInheritedOrigin;
}
//# sourceMappingURL=BrowserFrameURL.d.ts.map