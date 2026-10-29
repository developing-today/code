# ACP spawn: opencode @ /home/user/code

Command: `/acp spawn opencode --cwd /home/user/code`

- Backend: OpenClaw ACP runtime (config `acp.backend = "acpx"`, defaultAgent `opencode`).
- Result: the OpenClaw gateway launched `opencode acp` (pid 587787, parent = gateway pid 583752)
  and bridged it as session `agent:main:acp-bridge:5b5899aa-1a75-4834-af06-e0e4a1e5d68a`.
- Agent override (`~/.acpx/config.json`): `opencode -> /etc/profiles/per-user/user/bin/opencode acp`.

Delegated task: reply with the exact output of running `opencode --version`.

Verbatim output:

```
opencode v2.0.30+56590c5
```

Notes:

- First one-shot attempt failed: `MODEL_NOT_IN_PLAN: GPT-5.6 Terra ...` (OpenCode's default model
  is not in-plan on this host). Retried pinned to `commandcode/deepseek/deepseek-v4-flash` -> exit 0.
- Run: `acpx --cwd /home/user/code --format quiet --approve-all --model commandcode/deepseek/deepseek-v4-flash opencode exec "Reply with the exact output of running: opencode --version"`
- Raw run log: `acp-spawn-version.out`.
