// Fail-closed error type for fleet-hosted (#12583), mirroring the
// pkgs/id/hosted-worker idiom: every refusal in the auth/tenancy/router
// layers is a FailClosed carrying the HTTP status and a stable machine
// code, rendered by the router as { "error": code, "message": ... }.
// Nothing in this package fails open: a check that cannot complete
// throws, and the throw refuses the request.

export class FailClosed extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}
