# C007: Hierarchical Issue Handoff and Research Fanout

[Main reference](../main.md#c007)

## Relevant transcript excerpts

<!-- workflow:excerpt C007 35 37 -->

[Transcript lines 35-37](../transcript.txt#L35-L37)

```text
Then it should move on to the main task, which is that you need to review the whole document and generate an appropriate handoff for an agent to generate GitHub issues for all things to investigate or plan. This should be put into a GitHub issue that incorporates the concept of investigating and implementing some or all of the file in its entirety. On a later date an agent with zero context should be able to take the issue URL and create many GitHub issues covering all the facets to research and investigate. Each of those should be built as handoffs with all the details needed for a different agent to pick it up someday. This issue that was made can set up the basic design plan, which should have high-level stages. The high-level stages mostly boil down to covering just the research and design stage, where the design file will have a section for initial research stages.

These research stages will essentially be a list of every GitHub issue that was made or is related in any way to this topic, with a short description of each and roughly the priority order that recommends going about these stages. It should try to prevent blockers so that as much as possible can run in parallel on separate server agents. The purpose of this stage will be to create another handoff issue and that handoff issue's purpose is to orchestrate running subagents against each GitHub issue. It should also update the main reference doc and design to populate any new issues and PRs and add new details. It shouldn't remove the findings from earlier stages. It should mark its section afterwards and denote what was found.
```

## Normalized purpose

Generate self-contained parent and child GitHub issues for concepts and facets, establishing the initial research stages in design.md.

## Initial thoughts

- Parent issues define the overall goal; child issues isolate granular research facets.
- Issues must be self-contained so a zero-context agent can execute them without conversation history.

## Inferred suggestions

- Inference: Attach machine-readable markers (<!-- workflow:issue ... -->) to issues to avoid duplicate publishing.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
