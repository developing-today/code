# Guarded Python laptop client migration (#16088)

This is a source-only staging slice, not retirement or a deployment. The
`laptop-agent.py` executable validates its complete CLI before doing anything,
then probes the exact `fleet` found on PATH using `fleet capabilities`. The
probe is fresh on every invocation (no readiness cache), local-only, and bounded
to 2 seconds and 64 KiB. Missing binaries, probe failures, malformed JSON,
nonzero status, unavailable domains, and old capabilities all retain the frozen
`laptop-agent-legacy.py` implementation. No domain/version heuristic enables it.

## Producer contract — advertised exactly when the domain is available

`fleet capabilities` returns BOTH (code#16088, slice
`fleet/oc-code-06/26198-compat-marker`):

```json
{
  "compatibility_contracts": ["laptop-agent-python-cli/v1"],
  "domains": [{ "name": "laptop-agent", "available": true, "state": "available" }]
}
```

The marker is emitted if and only if the laptop-agent domain is `available`
to that invocation — narrowed profiles (`fleet-serve`, `fleet-connect`) and
config-disabled domains advertise an empty list. Availability alone never
advertises, and no version/domain heuristic enables the cutover.```

The marker promises the entire legacy Python CLI's semantics, not just verb
names: health; exec; exec-batch; run (including accepted `--timeout`); poll;
kill; jobs; fetch with `--chunk` and resume; command argument boundaries;
stdout/stderr/exit conventions; deadlines, cancellation and launch guards;
transport pins, fallback behavior and existing detached-job ownership/cursors.
The marker rests on the Rust CLI surface in `fleet-laptop-agent`
(parse + oracle-named crate tests: exec-batch, fetch `--chunk`/resume with
overshoot refusal, run's accepted `--timeout`) — not on a version or domain
heuristic. SSH fallback ownership is separately tracked in #16083. Live
parity against the deployed binary and legacy retirement remain on #16088.

With the exact marker and available domain, the selector replaces itself with
`<selected fleet> laptop-agent <original arguments...>` using exec, inheriting
stdin, stdout, stderr, environment and signals. There is **no fallback replay**
after selecting exec, including an exec failure (exit 126), avoiding duplicate
remote side effects. Probe output never leaks into command output. Validation
rejects unknown/abbreviated flags, stray arguments and missing values with exit
2; command payloads after `--` remain opaque. `--option=value` is accepted.

## Install the pair safely; retain import callers

No tracked bootstrap currently installs this client. Do not copy the new
entrypoint alone. From a complete source checkout use:

```sh
python3 pkgs/fleet/install-laptop-agent-client.py --dest "$HOME/tools"
```

The installer reads both sources before writing, publishes the legacy dependency
first, then atomically replaces the entrypoint (fsync for files and directory).
If interrupted before the second rename, the old self-contained client still
works. Keep the legacy filename and import API backward compatible on subsequent
installs. The broker's existing import of installed `laptop-agent.py` executes
the frozen legacy implementation in the imported module's namespace: constants,
private functions AND writes to module globals retain their old behavior. Import
callers never probe fleet. Neither the broker nor SSH helpers are removed.

The parent task's November 27 deployed probe found an older
`~/tools/laptop-agent.py` (25,292 bytes, November 14), with no
`laptop-agent-broker.py`, `laptop-ssh.sh` or `laptop-ssh-real.sh` on that laptop.
That is installation drift, not evidence of full live SSH parity; this slice
makes no live parity claim and performs no installation/deployment.

## Hermetic verification

```sh
python3 -I -m unittest discover -s pkgs/fleet/tests -p 'test_python_client_selector.py' -v
python3 -I -m unittest discover -s pkgs/fleet/tests -p 'test_tailnet_proxy_env.py' -v
```

Tests name their oracle: the explicit gate/CLI contract, fake fleet's exact
argv/bytes/status (all verbs, stdin and no replay), and dependency-first install
ordering including concurrent old-client imports. No real daemon or secrets are
required. Full retirement remains on #16088, pending producer parity and deployed
verification.
