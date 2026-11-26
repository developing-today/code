// A refusal with the HTTP status it should travel as. It has a module of
// its own so that code which runs at the edge can raise one without
// pulling in the host's process-running code.
export class OpError extends Error {
  constructor(
    public status: 400 | 404 | 409 | 502 | 503,
    message: string,
    /** More the answer carries beside the message, such as what a
     * refusal found in place of what was expected. */
    public said?: Record<string, unknown>,
  ) {
    super(message);
  }
}
