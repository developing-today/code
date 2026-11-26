// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="index.html"><strong aria-hidden="true">1.</strong> fleet</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="getting-started/installation.html"><strong aria-hidden="true">2.</strong> Installation and bootstrap</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="getting-started/quickstart.html"><strong aria-hidden="true">3.</strong> Quickstart</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="getting-started/shape.html"><strong aria-hidden="true">4.</strong> The shape of the binary</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/architecture.html"><strong aria-hidden="true">5.</strong> Architecture</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/queue.html"><strong aria-hidden="true">6.</strong> The queue: tasks, lanes and leases</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/placement.html"><strong aria-hidden="true">7.</strong> Placement and remote dispatch</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/secrets.html"><strong aria-hidden="true">8.</strong> Secrets and the secrets broker</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/bridge.html"><strong aria-hidden="true">9.</strong> The bridge: fleet messaging</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/agents.html"><strong aria-hidden="true">10.</strong> Agents: enrolment, doctor and the intake ladder</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/proxy.html"><strong aria-hidden="true">11.</strong> Proxy, sessions and recovery</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/commander.html"><strong aria-hidden="true">12.</strong> The Commander door: Desktop Commander over MCP</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/search.html"><strong aria-hidden="true">13.</strong> Search: the Atlas corpus</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/gh.html"><strong aria-hidden="true">14.</strong> The gh passthrough</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/dispatch.html"><strong aria-hidden="true">15.</strong> Dispatch and dispatch-test</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/fleet-hosted.html"><strong aria-hidden="true">16.</strong> The hosted edition: fleet-hosted</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/queue-transport-alert.html"><strong aria-hidden="true">17.</strong> The queue watchdog: queue-transport-alert</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/dispatch-jobs-store.html"><strong aria-hidden="true">18.</strong> The dispatch-test jobs store: fleet-dispatch-jobs</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/temporary-builds.html"><strong aria-hidden="true">19.</strong> Temporary builds and custom connections</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/compile-coordinator.html"><strong aria-hidden="true">20.</strong> The central compile coordinator</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="concepts/lab-cross-links.html"><strong aria-hidden="true">21.</strong> Cross-links: automation, Cloudflare pipes and DNS</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/builtins.html"><strong aria-hidden="true">22.</strong> Built-in commands</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/queue-core.html"><strong aria-hidden="true">23.</strong> fleet queue: core verbs</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/queue-lifecycle.html"><strong aria-hidden="true">24.</strong> fleet queue: lifecycle verbs</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/queue-fallback.html"><strong aria-hidden="true">25.</strong> fleet queue: the fallback queue</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/queue-dispatch.html"><strong aria-hidden="true">26.</strong> fleet queue: dispatch and sessions</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/queue-admin.html"><strong aria-hidden="true">27.</strong> fleet queue: inspection and admin</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/serve.html"><strong aria-hidden="true">28.</strong> fleet serve</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/proxy.html"><strong aria-hidden="true">29.</strong> fleet proxy</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/dispatch.html"><strong aria-hidden="true">30.</strong> fleet dispatch</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/dispatch-test.html"><strong aria-hidden="true">31.</strong> fleet dispatch-test</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/secrets.html"><strong aria-hidden="true">32.</strong> fleet secrets</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/gh.html"><strong aria-hidden="true">33.</strong> fleet gh</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/bridge.html"><strong aria-hidden="true">34.</strong> fleet bridge</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/agent.html"><strong aria-hidden="true">35.</strong> fleet agent</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/shell.html"><strong aria-hidden="true">36.</strong> fleet shell</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/search.html"><strong aria-hidden="true">37.</strong> fleet search</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/orgs.html"><strong aria-hidden="true">38.</strong> fleet orgs</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/cert.html"><strong aria-hidden="true">39.</strong> fleet cert</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/laptop-agent.html"><strong aria-hidden="true">40.</strong> fleet laptop-agent</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/laptop-route-forms.html"><strong aria-hidden="true">41.</strong> fleet laptop route forms</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="reference/actions.html"><strong aria-hidden="true">42.</strong> fleet actions</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="tutorials/enrol-a-seat.html"><strong aria-hidden="true">43.</strong> Tutorial: enrol a new seat</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="tutorials/claim-and-complete.html"><strong aria-hidden="true">44.</strong> Tutorial: claim and complete a queue task</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="tutorials/dispatch-a-build.html"><strong aria-hidden="true">45.</strong> Tutorial: dispatch a build remotely</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="tutorials/resolve-a-secret.html"><strong aria-hidden="true">46.</strong> Tutorial: resolve a secret</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="tutorials/bridge-messaging.html"><strong aria-hidden="true">47.</strong> Tutorial: bridge messaging</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="tutorials/run-the-brokers.html"><strong aria-hidden="true">48.</strong> Tutorial: run the brokers</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="library/crates.html"><strong aria-hidden="true">49.</strong> The crates</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="library/queue-client.html"><strong aria-hidden="true">50.</strong> The queue client</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="library/placement.html"><strong aria-hidden="true">51.</strong> Placement and surfaces as data</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="library/registry-and-config.html"><strong aria-hidden="true">52.</strong> Registry, config and capabilities</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/layout.html"><strong aria-hidden="true">53.</strong> Repository layout</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/building.html"><strong aria-hidden="true">54.</strong> Building fleet</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/testing.html"><strong aria-hidden="true">55.</strong> Testing</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/add-a-subcommand.html"><strong aria-hidden="true">56.</strong> Add a subcommand</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/add-a-flag.html"><strong aria-hidden="true">57.</strong> Add a flag</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/register-a-domain.html"><strong aria-hidden="true">58.</strong> Register a domain or capability</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/release-and-rollout.html"><strong aria-hidden="true">59.</strong> Release and rollout</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/keeping-this-book-current.html"><strong aria-hidden="true">60.</strong> Keeping this book current</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/program-roadmap.html"><strong aria-hidden="true">61.</strong> Fleet research and implementation roadmap</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/high-level-roadmap.html"><strong aria-hidden="true">62.</strong> High-level roadmap</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/queue-placement-seats.html"><strong aria-hidden="true">63.</strong> Queue, placement and seat contracts</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/brokers-builds-releases.html"><strong aria-hidden="true">64.</strong> Brokers, remote builds and the release train</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="development/quota-status-integrations.html"><strong aria-hidden="true">65.</strong> Quota, operator visibility and integration contracts</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/program-issue-map.html"><strong aria-hidden="true">66.</strong> Open Fleet issues and prerequisites</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/services.html"><strong aria-hidden="true">67.</strong> Running the brokers as services</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/health-and-healing.html"><strong aria-hidden="true">68.</strong> Health checks and the heal stack</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/troubleshooting.html"><strong aria-hidden="true">69.</strong> Troubleshooting</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/deploying-this-book.html"><strong aria-hidden="true">70.</strong> Deploying this book</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/faq.html"><strong aria-hidden="true">71.</strong> FAQ</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="operations/glossary.html"><strong aria-hidden="true">72.</strong> Glossary</a></span></li><li class="chapter-item expanded "><li class="part-title">Research and development</li></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/overview.html"><strong aria-hidden="true">73.</strong> Fleet research and development</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/roadmap.html"><strong aria-hidden="true">74.</strong> Delivery plan and evidence gates</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/queue-placement.html"><strong aria-hidden="true">75.</strong> Queue guarantees and dispatch decisions</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/seats-and-recovery.html"><strong aria-hidden="true">76.</strong> Seat identity, credential readiness and channel recovery</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/operational-verification.html"><strong aria-hidden="true">77.</strong> Operational verification</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/builds-and-releases.html"><strong aria-hidden="true">78.</strong> Remote verification, fork convergence and releases</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/quota-status-integration.html"><strong aria-hidden="true">79.</strong> Quota decisions, operator status and service contracts</a></span></li><li class="chapter-item expanded "><span class="chapter-link-wrapper"><a href="research/issue-map.html"><strong aria-hidden="true">80.</strong> Fleet issue map for November 23, 2026</a></span></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split('#')[0].split('?')[0];
        if (current_page.endsWith('/')) {
            current_page += 'index.html';
        }
        const links = Array.prototype.slice.call(this.querySelectorAll('a'));
        const l = links.length;
        for (let i = 0; i < l; ++i) {
            const link = links[i];
            const href = link.getAttribute('href');
            if (href && !href.startsWith('#') && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The 'index' page is supposed to alias the first chapter in the book.
            // Check both with and without the '.html' suffix to be robust against pretty URLs
            if (link.href.replace(/\.html$/, '') === current_page.replace(/\.html$/, '')
                || i === 0
                && path_to_root === ''
                && current_page.endsWith('/index.html')) {
                link.classList.add('active');
                let parent = link.parentElement;
                while (parent) {
                    if (parent.tagName === 'LI' && parent.classList.contains('chapter-item')) {
                        parent.classList.add('expanded');
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', e => {
            if (e.target.tagName === 'A') {
                const clientRect = e.target.getBoundingClientRect();
                const sidebarRect = this.getBoundingClientRect();
                sessionStorage.setItem('sidebar-scroll-offset', clientRect.top - sidebarRect.top);
            }
        }, { passive: true });
        const sidebarScrollOffset = sessionStorage.getItem('sidebar-scroll-offset');
        sessionStorage.removeItem('sidebar-scroll-offset');
        if (sidebarScrollOffset !== null) {
            // preserve sidebar scroll position when navigating via links within sidebar
            const activeSection = this.querySelector('.active');
            if (activeSection) {
                const clientRect = activeSection.getBoundingClientRect();
                const sidebarRect = this.getBoundingClientRect();
                const currentOffset = clientRect.top - sidebarRect.top;
                this.scrollTop += currentOffset - parseFloat(sidebarScrollOffset);
            }
        } else {
            // scroll sidebar to current active section when navigating via
            // 'next/previous chapter' buttons
            const activeSection = document.querySelector('#mdbook-sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        const sidebarAnchorToggles = document.querySelectorAll('.chapter-fold-toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(el => {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define('mdbook-sidebar-scrollbox', MDBookSidebarScrollbox);


// ---------------------------------------------------------------------------
// Support for dynamically adding headers to the sidebar.

(function() {
    // This is used to detect which direction the page has scrolled since the
    // last scroll event.
    let lastKnownScrollPosition = 0;
    // This is the threshold in px from the top of the screen where it will
    // consider a header the "current" header when scrolling down.
    const defaultDownThreshold = 150;
    // Same as defaultDownThreshold, except when scrolling up.
    const defaultUpThreshold = 300;
    // The threshold is a virtual horizontal line on the screen where it
    // considers the "current" header to be above the line. The threshold is
    // modified dynamically to handle headers that are near the bottom of the
    // screen, and to slightly offset the behavior when scrolling up vs down.
    let threshold = defaultDownThreshold;
    // This is used to disable updates while scrolling. This is needed when
    // clicking the header in the sidebar, which triggers a scroll event. It
    // is somewhat finicky to detect when the scroll has finished, so this
    // uses a relatively dumb system of disabling scroll updates for a short
    // time after the click.
    let disableScroll = false;
    // Array of header elements on the page.
    let headers;
    // Array of li elements that are initially collapsed headers in the sidebar.
    // I'm not sure why eslint seems to have a false positive here.
    // eslint-disable-next-line prefer-const
    let headerToggles = [];
    // This is a debugging tool for the threshold which you can enable in the console.
    let thresholdDebug = false;

    // Updates the threshold based on the scroll position.
    function updateThreshold() {
        const scrollTop = window.pageYOffset || document.documentElement.scrollTop;
        const windowHeight = window.innerHeight;
        const documentHeight = document.documentElement.scrollHeight;

        // The number of pixels below the viewport, at most documentHeight.
        // This is used to push the threshold down to the bottom of the page
        // as the user scrolls towards the bottom.
        const pixelsBelow = Math.max(0, documentHeight - (scrollTop + windowHeight));
        // The number of pixels above the viewport, at least defaultDownThreshold.
        // Similar to pixelsBelow, this is used to push the threshold back towards
        // the top when reaching the top of the page.
        const pixelsAbove = Math.max(0, defaultDownThreshold - scrollTop);
        // How much the threshold should be offset once it gets close to the
        // bottom of the page.
        const bottomAdd = Math.max(0, windowHeight - pixelsBelow - defaultDownThreshold);
        let adjustedBottomAdd = bottomAdd;

        // Adjusts bottomAdd for a small document. The calculation above
        // assumes the document is at least twice the windowheight in size. If
        // it is less than that, then bottomAdd needs to be shrunk
        // proportional to the difference in size.
        if (documentHeight < windowHeight * 2) {
            const maxPixelsBelow = documentHeight - windowHeight;
            const t = 1 - pixelsBelow / Math.max(1, maxPixelsBelow);
            const clamp = Math.max(0, Math.min(1, t));
            adjustedBottomAdd *= clamp;
        }

        let scrollingDown = true;
        if (scrollTop < lastKnownScrollPosition) {
            scrollingDown = false;
        }

        if (scrollingDown) {
            // When scrolling down, move the threshold up towards the default
            // downwards threshold position. If near the bottom of the page,
            // adjustedBottomAdd will offset the threshold towards the bottom
            // of the page.
            const amountScrolledDown = scrollTop - lastKnownScrollPosition;
            const adjustedDefault = defaultDownThreshold + adjustedBottomAdd;
            threshold = Math.max(adjustedDefault, threshold - amountScrolledDown);
        } else {
            // When scrolling up, move the threshold down towards the default
            // upwards threshold position. If near the bottom of the page,
            // quickly transition the threshold back up where it normally
            // belongs.
            const amountScrolledUp = lastKnownScrollPosition - scrollTop;
            const adjustedDefault = defaultUpThreshold - pixelsAbove
                + Math.max(0, adjustedBottomAdd - defaultDownThreshold);
            threshold = Math.min(adjustedDefault, threshold + amountScrolledUp);
        }

        if (documentHeight <= windowHeight) {
            threshold = 0;
        }

        if (thresholdDebug) {
            const id = 'mdbook-threshold-debug-data';
            let data = document.getElementById(id);
            if (data === null) {
                data = document.createElement('div');
                data.id = id;
                data.style.cssText = `
                    position: fixed;
                    top: 50px;
                    right: 10px;
                    background-color: 0xeeeeee;
                    z-index: 9999;
                    pointer-events: none;
                `;
                document.body.appendChild(data);
            }
            data.innerHTML = `
                <table>
                  <tr><td>documentHeight</td><td>${documentHeight.toFixed(1)}</td></tr>
                  <tr><td>windowHeight</td><td>${windowHeight.toFixed(1)}</td></tr>
                  <tr><td>scrollTop</td><td>${scrollTop.toFixed(1)}</td></tr>
                  <tr><td>pixelsAbove</td><td>${pixelsAbove.toFixed(1)}</td></tr>
                  <tr><td>pixelsBelow</td><td>${pixelsBelow.toFixed(1)}</td></tr>
                  <tr><td>bottomAdd</td><td>${bottomAdd.toFixed(1)}</td></tr>
                  <tr><td>adjustedBottomAdd</td><td>${adjustedBottomAdd.toFixed(1)}</td></tr>
                  <tr><td>scrollingDown</td><td>${scrollingDown}</td></tr>
                  <tr><td>threshold</td><td>${threshold.toFixed(1)}</td></tr>
                </table>
            `;
            drawDebugLine();
        }

        lastKnownScrollPosition = scrollTop;
    }

    function drawDebugLine() {
        if (!document.body) {
            return;
        }
        const id = 'mdbook-threshold-debug-line';
        const existingLine = document.getElementById(id);
        if (existingLine) {
            existingLine.remove();
        }
        const line = document.createElement('div');
        line.id = id;
        line.style.cssText = `
            position: fixed;
            top: ${threshold}px;
            left: 0;
            width: 100vw;
            height: 2px;
            background-color: red;
            z-index: 9999;
            pointer-events: none;
        `;
        document.body.appendChild(line);
    }

    function mdbookEnableThresholdDebug() {
        thresholdDebug = true;
        updateThreshold();
        drawDebugLine();
    }

    window.mdbookEnableThresholdDebug = mdbookEnableThresholdDebug;

    // Updates which headers in the sidebar should be expanded. If the current
    // header is inside a collapsed group, then it, and all its parents should
    // be expanded.
    function updateHeaderExpanded(currentA) {
        // Add expanded to all header-item li ancestors.
        let current = currentA.parentElement;
        while (current) {
            if (current.tagName === 'LI' && current.classList.contains('header-item')) {
                current.classList.add('expanded');
            }
            current = current.parentElement;
        }
    }

    // Updates which header is marked as the "current" header in the sidebar.
    // This is done with a virtual Y threshold, where headers at or below
    // that line will be considered the current one.
    function updateCurrentHeader() {
        if (!headers || !headers.length) {
            return;
        }

        // Reset the classes, which will be rebuilt below.
        const els = document.getElementsByClassName('current-header');
        for (const el of els) {
            el.classList.remove('current-header');
        }
        for (const toggle of headerToggles) {
            toggle.classList.remove('expanded');
        }

        // Find the last header that is above the threshold.
        let lastHeader = null;
        for (const header of headers) {
            const rect = header.getBoundingClientRect();
            if (rect.top <= threshold) {
                lastHeader = header;
            } else {
                break;
            }
        }
        if (lastHeader === null) {
            lastHeader = headers[0];
            const rect = lastHeader.getBoundingClientRect();
            const windowHeight = window.innerHeight;
            if (rect.top >= windowHeight) {
                return;
            }
        }

        // Get the anchor in the summary.
        const href = '#' + lastHeader.id;
        const a = [...document.querySelectorAll('.header-in-summary')]
            .find(element => element.getAttribute('href') === href);
        if (!a) {
            return;
        }

        a.classList.add('current-header');

        updateHeaderExpanded(a);
    }

    // Updates which header is "current" based on the threshold line.
    function reloadCurrentHeader() {
        if (disableScroll) {
            return;
        }
        updateThreshold();
        updateCurrentHeader();
    }


    // When clicking on a header in the sidebar, this adjusts the threshold so
    // that it is located next to the header. This is so that header becomes
    // "current".
    function headerThresholdClick(event) {
        // See disableScroll description why this is done.
        disableScroll = true;
        setTimeout(() => {
            disableScroll = false;
        }, 100);
        // requestAnimationFrame is used to delay the update of the "current"
        // header until after the scroll is done, and the header is in the new
        // position.
        requestAnimationFrame(() => {
            requestAnimationFrame(() => {
                // Closest is needed because if it has child elements like <code>.
                const a = event.target.closest('a');
                const href = a.getAttribute('href');
                const targetId = href.substring(1);
                const targetElement = document.getElementById(targetId);
                if (targetElement) {
                    threshold = targetElement.getBoundingClientRect().bottom;
                    updateCurrentHeader();
                }
            });
        });
    }

    // Takes the nodes from the given head and copies them over to the
    // destination, along with some filtering.
    function filterHeader(source, dest) {
        const clone = source.cloneNode(true);
        clone.querySelectorAll('mark').forEach(mark => {
            mark.replaceWith(...mark.childNodes);
        });
        dest.append(...clone.childNodes);
    }

    // Scans page for headers and adds them to the sidebar.
    document.addEventListener('DOMContentLoaded', function() {
        const activeSection = document.querySelector('#mdbook-sidebar .active');
        if (activeSection === null) {
            return;
        }

        const main = document.getElementsByTagName('main')[0];
        headers = Array.from(main.querySelectorAll('h2, h3, h4, h5, h6'))
            .filter(h => h.id !== '' && h.children.length && h.children[0].tagName === 'A');

        if (headers.length === 0) {
            return;
        }

        // Build a tree of headers in the sidebar.

        const stack = [];

        const firstLevel = parseInt(headers[0].tagName.charAt(1));
        for (let i = 1; i < firstLevel; i++) {
            const ol = document.createElement('ol');
            ol.classList.add('section');
            if (stack.length > 0) {
                stack[stack.length - 1].ol.appendChild(ol);
            }
            stack.push({level: i + 1, ol: ol});
        }

        // The level where it will start folding deeply nested headers.
        const foldLevel = 3;

        for (let i = 0; i < headers.length; i++) {
            const header = headers[i];
            const level = parseInt(header.tagName.charAt(1));

            const currentLevel = stack[stack.length - 1].level;
            if (level > currentLevel) {
                // Begin nesting to this level.
                for (let nextLevel = currentLevel + 1; nextLevel <= level; nextLevel++) {
                    const ol = document.createElement('ol');
                    ol.classList.add('section');
                    const last = stack[stack.length - 1];
                    const lastChild = last.ol.lastChild;
                    // Handle the case where jumping more than one nesting
                    // level, which doesn't have a list item to place this new
                    // list inside of.
                    if (lastChild) {
                        lastChild.appendChild(ol);
                    } else {
                        last.ol.appendChild(ol);
                    }
                    stack.push({level: nextLevel, ol: ol});
                }
            } else if (level < currentLevel) {
                while (stack.length > 1 && stack[stack.length - 1].level > level) {
                    stack.pop();
                }
            }

            const li = document.createElement('li');
            li.classList.add('header-item');
            li.classList.add('expanded');
            if (level < foldLevel) {
                li.classList.add('expanded');
            }
            const span = document.createElement('span');
            span.classList.add('chapter-link-wrapper');
            const a = document.createElement('a');
            span.appendChild(a);
            a.href = '#' + header.id;
            a.classList.add('header-in-summary');
            filterHeader(header.children[0], a);
            a.addEventListener('click', headerThresholdClick);
            const nextHeader = headers[i + 1];
            if (nextHeader !== undefined) {
                const nextLevel = parseInt(nextHeader.tagName.charAt(1));
                if (nextLevel > level && level >= foldLevel) {
                    const toggle = document.createElement('a');
                    toggle.classList.add('chapter-fold-toggle');
                    toggle.classList.add('header-toggle');
                    toggle.addEventListener('click', () => {
                        li.classList.toggle('expanded');
                    });
                    const toggleDiv = document.createElement('div');
                    toggleDiv.textContent = '❱';
                    toggle.appendChild(toggleDiv);
                    span.appendChild(toggle);
                    headerToggles.push(li);
                }
            }
            li.appendChild(span);

            const currentParent = stack[stack.length - 1];
            currentParent.ol.appendChild(li);
        }

        const onThisPage = document.createElement('div');
        onThisPage.classList.add('on-this-page');
        onThisPage.append(stack[0].ol);
        const activeItemSpan = activeSection.parentElement;
        activeItemSpan.after(onThisPage);
    });

    document.addEventListener('DOMContentLoaded', reloadCurrentHeader);
    document.addEventListener('scroll', reloadCurrentHeader, { passive: true });
})();

