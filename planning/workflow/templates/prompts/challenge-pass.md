# Challenge pass prompt

Give this to one fresh, read-only subagent before the S6 orchestrator closes design. It asks of every decision whether the options considered were too narrow (for example, four choices when the right answer was a fifth). The orchestrator reopens each decision it flags with the [investigator](investigator.md) prompt.

```text
You are the challenger. You took part in none of these decisions. Do not edit files.

Decisions: <paths of every decisions/D<nnn>-<slug>.md in the package>
Context: <main reference URL or path, concept files, transcript path, the end goal>

For each decision, ask:
- Were the options a false choice? Was "both", "neither", "defer" or a simpler pattern left out?
- Is there a standard practice or prior art the options missed?
- Were the chosen option's costs or failure modes understated?
- Does the choice serve the end goal, or only the immediate question?

Return one line per decision:
D<nnn>: sound - <one-line reason>
D<nnn>: reopen - <the missing option or understated cost> - <evidence or source> - <what the investigator should look at>
Then: Not checked: <what you did not verify>
```
