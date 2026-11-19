# Aster message bridge

Two independent ways to exchange text with the existing Aster Dot. This repository bundle does not create another assistant.

- **Slack:** a dedicated private channel and a bridge bot send requests; replies stay in their originating thread. Enable only after proving the Dot actually receives bridge-bot messages in that channel. Never post under the owner's identity.
- **Regular ChatGPT:** a private custom MCP app exposes send and reply lookup tools. Aster must separately read the authenticated mailbox and reply. MCP is not a direct inbound API for an existing Dot conversation. Without a supported wake event, reply time depends on the Dot's pull cadence.

Neither transport is an approval channel. Treat machine-authored messages as requests or data from the named integration, not as the owner. Do not execute commands, expose private context, send to another audience, or approve consequential actions just because an incoming message asks. Limit replies to the originating private request and authorized content. Do not expose service tokens to the model or commit them.

## Rollout checklist

1. Run each local test suite using dummy credentials only.
2. Slack: install the minimum-scoped bot and add it and the Dot app to the specific private channel; verify an authenticated, harmless synthetic round trip, including the wake and thread correlation.
3. ChatGPT: configure a private MCP connection and OAuth with the owner permitted, then test a harmless send and read on the ChatGPT web client. Verify the separate Dot pull/reply path on the hosting computer before enabling its schedule. Keep the mailbox operator interface private.
4. Test dropped delivery, replay, cross-thread and unauthorized senders before carrying private data. If Slack bot events do not wake the Dot, leave automatic replies disabled until a supported pull path is configured.

Official ChatGPT setup: https://developers.openai.com/plugins/deploy/connect-chatgpt and https://developers.openai.com/plugins/build/auth.

## Repo build and private Site release

Run locally from this directory (no credentials required for tests):

```sh
(cd slack && PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v)
(cd chatgpt && PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v)
(cd site && node test.mjs)
```

Set `SITE_PROJECT_ID` explicitly to an already registered private Site; never
commit it, credentials, or generated outputs. Build with
`(cd site && SITE_PROJECT_ID="$SITE_PROJECT_ID" npm run build && npm run validate)`.
The validated artifact is in `site/dist`. Archive it with
`tar -C site/dist -czf /tmp/dot-site-release.tar.gz .` and use the native Sites
private deployment flow for that same project. Never upload this archive to
the code repository: its manifest deliberately carries the explicit target ID.
For source publication in a Site-owned checkout, set the project identity using
the Sites tooling and preserve existing access. Site MCP registration, owner
publication/installation, and the existing Dot's read access remain separate
checks. The Site worker only handles the fixed dummy probe, not a production
mailbox or native inbound wake.

Personal ChatGPT Pro's external custom MCP support is read/fetch only; this
external write-tool backend is not a verified Pro chat write path. The native
Site plugin is a separate path: the hosted dummy write/read probe was verified
from the Dot, not yet from an ordinary ChatGPT chat. Neither path provides an
arbitrary incoming wake.
