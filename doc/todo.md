bunx oh-my-openagent install
bunx oh-my-opencode install --no-tui --claude=no --openai=no --gemini=no --copilot=yes --opencode-go=no --opencode-zen=no --zai-coding-plan=no
https://github.com/kdcokenny/ocx
ocx add kdco/workspace --from https://registry.kdco.dev
ocx add kdco/worktree --from https://registry.kdco.dev
ocx add kdco/background-agents --from https://registry.kdco.dev
ocx add kdco/notify --from https://registry.kdco.dev
https://github.com/NeuralNomadsAI/CodeNomad
https://github.com/morapelker/hive
bun add -g btca opencode-ai
btca connect --provider opencode --model claude-haiku-4-5
---
https://github.com/fishfolk/bones
https://github.com/rustonbsd/rustpatcher
https://github.com/p2panda/p2panda
https://github.com/HIRO-MicroDataCenters-BV/rhio

https://github.com/anomalyco/opencode/pull/8721

curl -s -H "Authorization: Bearer $(cat ~/.config/github-copilot/apps.json \
| jq '.[].oauth_token' \
| sed 's/^"\(.*\)"$/\1/')" https://api.github.com/copilot_internal/user
