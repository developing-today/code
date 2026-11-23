// One function, with nothing else in its file so that anything may import it.

/** A function that calls `f` plainly, for keeping in place of `f`.
 *
 * The Worker runtime's fetch refuses to be called as a method of
 * anything else: `this.fetchImpl(url)`, with the platform's fetch kept
 * in `fetchImpl`, throws "Illegal invocation" there, and only there (bun
 * and Node do not mind, so nothing run under them notices). A function
 * kept on an object is called that way sooner or later; for a day every
 * notification the edge sent, and every read of the Android release,
 * failed on it. So nothing here keeps a fetch it was given: it keeps
 * this. */
export function plain<A extends unknown[], R>(f: (...args: A) => R): (...args: A) => R {
  return (...args: A) => f(...args);
}
