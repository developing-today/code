/**
 * Cookie domain validation.
 */
export default class CookieDomainUtility {
    /**
     * Normalizes a domain attribute and validates it against the setting host.
     *
     * @param domain Domain attribute.
     * @param hostname Setting host.
     * @returns Domain, an empty string for host-only cookies, or null if invalid.
     */
    static validateDomain(domain: string, hostname: string): string | null;
    /**
     * Returns whether a normalized domain is a public suffix.
     *
     * @param domain ASCII domain.
     * @returns Whether it is a public suffix.
     */
    static isPublicSuffix(domain: string): boolean;
    /**
     * Returns whether a hostname equals a domain or is its DNS subdomain.
     *
     * @param hostname Hostname.
     * @param domain Domain.
     * @returns Whether the domain matches.
     */
    static domainMatches(hostname: string | undefined, domain: string): boolean;
}
//# sourceMappingURL=CookieDomainUtility.d.ts.map