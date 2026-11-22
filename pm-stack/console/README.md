# pm-console

The write side of the PM stack, and the gate in front of it. The
dashboard (`pm-aggregator`) shows the fleet; the console acts on it, from
a phone or a browser, at `https://pm.security.cab/console/`.

The whole public hostname is routed here. `/console` is this service.
Every other path is the dashboard's, and the console passes it through
only for a verified owner: the dashboard has no sign-in of its own, so
the check is made at this origin as well as at the edge, and does not
depend on how the edge reads a path. Writes from another site are
refused. Five files a phone fetches before anyone has signed in are
public, matched exactly: `/manifest.webmanifest` and the four icons under
`/icons/`.

| Section       | What you can do                                                                     | What runs                               |
| ------------- | ----------------------------------------------------------------------------------- | --------------------------------------- |
| Overview      | Queue counts, ready pull requests, agents, failed units                             | the calls below, together               |
| Queue         | List queued, claimed, blocked and parked tasks; unblock or cancel one with a reason | the queue store (read), `fleet queue`   |
| Priorities    | Ranked issues, most urgent first; change a rank; comment; file new work             | `fleet gh`                              |
| Agents        | Send an idea, message or command to one agent, a group or everyone; recent messages | `fleet bridge`                          |
| Services      | State of the stack's units; start, stop, restart; read logs                         | `systemctl --user`, `journalctl --user` |
| Pull requests | Open pull requests; check one; merge it                                             | `fleet gh`                              |

Every operation is an argument vector, never a shell line, and text you
type travels on stdin or as one argument.

Each write leaves two lines in `~/.local/state/pm-console/audit.jsonl`
(who, what, when; message text is not copied): one before the action
runs, one with how it went. If the first cannot be written the action is
not taken. If the second cannot be written the action's own result still
comes back, so a merge that happened is never reported as failed.

Work filed from Priorities is created as an issue and enqueued as a
triage task in the same pass, so a worker picks it up without waiting for
the coverage reconciliation. The rank sets the task's place in the queue.
If the enqueue fails, the issue still exists and the page says it is not
queued.

## Sign-in

The console runs no sign-in of its own. A caller arrives with a signed
token from an issuer the deployment trusts; the console checks the
signature, issuer, audience and times, then asks whether the identity is
the owner (`PM_CONSOLE_OWNER_EMAILS`).

- **Today:** Cloudflare Access at the public edge runs the interactive
  sign-in and issues the token. The sign-in methods are set on the Access
  application, not here.
- **Google:** an Access sign-in method. It turns on when
  `scripts/expose.py` finds the secrets `google.access-sso.client-id` and
  `google.access-sso.client-secret` (an OAuth client whose redirect URI is
  `https://<team>.cloudflareaccess.com/cdn-cgi/access/callback`).
- **The central OIDC provider (#9077):** either an Access sign-in method
  (generic OIDC), or trusted directly: add
  `{name, issuer, jwksUri, audience}` to the file named by
  `PM_CONSOLE_ISSUERS_FILE` and its ID tokens are accepted as
  `Authorization: Bearer`. RS256 and ES256.
- **Certificates from the central CA (#9155):** upload the CA to Access
  mTLS; the token then carries the certificate's common name, and names
  listed in `PM_CONSOLE_CERT_NAMES` are the owner.

A request is local, and treated as the owner, only when the connection
itself comes from this machine and it crossed no proxy (loopback Host, no
forwarding headers). The connection's address is the test, since headers
are the caller's to write; for the same reason the console only listens
on loopback and refuses to start on any other address. Everything else
needs a token. Outside the install files above, two paths answer without
one: `/console/health` and `/.well-known/assetlinks.json` (the Android
app's link to this origin).

## Run

```bash
bash pm-stack/console/install.sh          # user unit pm-console, port 4872, loopback only
python3 pm-stack/console/scripts/expose.py --check   # what public exposure would change
python3 pm-stack/console/scripts/expose.py           # Access, console env, tunnel route, DNS
cd pm-stack/console && bun test
```

`expose.py` is safe to run again: it changes only what is not already so
and refuses to touch DNS records or Access policies it did not write. It
puts the sign-in gate in place first and the route last, and stops before
the route if the console does not come up with its sign-in settings.

## Configuration

| Variable                                                  | Default                                           | Meaning                                            |
| --------------------------------------------------------- | ------------------------------------------------- | -------------------------------------------------- |
| `PM_CONSOLE_PORT`, `PM_CONSOLE_BIND`                      | `4872`, `127.0.0.1`                               | Listener; the address must be loopback             |
| `PM_CONSOLE_OWNER_EMAILS`                                 | none (nobody)                                     | Comma-separated owner addresses                    |
| `PM_CONSOLE_ACCESS_TEAM_DOMAIN`, `PM_CONSOLE_ACCESS_AUD`  | unset                                             | The Access issuer; set together                    |
| `PM_CONSOLE_ISSUERS_FILE`                                 | unset                                             | JSON list of further issuers                       |
| `PM_CONSOLE_CERT_NAMES`                                   | none                                              | Certificate common names that are the owner        |
| `PM_CONSOLE_DASHBOARD_URL`                                | `http://127.0.0.1:4870`                           | The dashboard behind the gate; loopback, or empty  |
| `PM_CONSOLE_UNITS`, `PM_CONSOLE_PROTECTED_UNITS`          | the stack's units                                 | Units the console manages; units it will not stop  |
| `PM_CONSOLE_ANDROID_PACKAGE`, `PM_CONSOLE_ANDROID_SHA256` | `today.developing.pmconsole`, none                | The Android app's identity in the asset-links file |
| `PM_CONSOLE_REPO`, `PM_CONSOLE_QUEUE`                     | `developing-today/code`, `repo-health-code-pilot` | What it operates on                                |

A malformed port, a listen address or dashboard address that is not
loopback, or a malformed issuer stops the process at boot and names the
variable.

## Android

`pm-stack/console/android/` is a Trusted Web Activity: the same site in
Chrome without browser chrome, so sign-in is Chrome's own and nothing is
reimplemented. The signed APK is built by the `pm-console-android`
workflow and attached to the release of the same name. See its README.
