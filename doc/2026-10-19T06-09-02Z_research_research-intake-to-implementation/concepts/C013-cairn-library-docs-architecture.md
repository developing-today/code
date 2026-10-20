# C013: Cairn Upstream Library, Generic Reusability, and Docs Architecture

[Main reference](../main.md#c013)

## Relevant transcript excerpts

<!-- workflow:excerpt C013 43 43 -->

[Transcript lines 43-43](../transcript.txt#L43-L43)

```text
Review the repo, and the docs already exist. Update agents.md however you need to and then create the skills to do each of the above, as well as to appropriately orchestrate the full situation (determining the current phase and making appropriate use of subagents).
```

<!-- workflow:excerpt C013 69 71 -->

[Transcript lines 69-71](../transcript.txt#L69-L71)

```text
clone dezren39 cairn down into git and review all files and skills. let's some useful bits from that influence the skills and doc structure etc. make some core starting docs that all meta main ref and design and feature docs may link to depending. design the architecture for docs in this repo what things are repo root or docs root, what deserves a folder and so. not hard rules or, rather, not immutable rules but should be followed unless good reason. update any docs or agents files or skills you want, make more than what I asked for if it helps. I kind of rambled but there's a clear set of steps from one or more short paragraphs to a series of issues for each concept along docs and basic design starts, to a series of issues for each concept which research and gather all information and do whatever is needed for a single subsection of each concepts design plan. following that a orchestrator of as many parallel agents whenever possible to execute all the design issues,handle cleanup merge validate the design, and make decisions that maybe subagents deferred, launch review and critique prompts, ensure the design is solid. maybe there were 4 choices and the task which decided between the options thought to narrowly and this is the one which pushes back and triggers investigator prompts. once it's satisfied, it will make an issue for the feature impl plan which granularly lays out the details and architecture steps and any blocks or decisions that depend on impl details once further along really building.  after the granular plan is built with appropriate issues that note blocks and prereqs, a handoff issue for orchestrating running the issues to full impl. I may have missed steps or misspoke you get the idea though,ale skills and docs and framework for this along with updating existing docs agents Readme and making a home for a variety of different standard and nonstandard docs and designs, plans, references.

review dezren39/cairn for direction and referemce/guidance. review all the files but you don't have to follow blindly, take what's right. you can add to any of these docs. put this raw text exactly into the appropriate place in cairn and follow the skills for that repo to make the skills for beginning to end of this process, bring the skills into this repo,customize however is appropriate for this repos context using the appropriate cairn skills. update the existing skills if appropriate make new ones if appropriate, you can edit any file in either repo. if you can make agents.md better do it. continue until you are prepared to accept a complex multipage document of assorted ideas and ramblings with typos and bad tts transcription. like this message, ideas are general but can and should be expanded made coherent. early skills are for adding detail organizing including parts that would be good but weren't well formed or only hinted at or were somehow missed but you think at minimum should be included in the reference corpus and considered. don't lose any things at this point refining is for later in design feature and impl, after detailed research of all options and possibilities, use cases, concerns, the full picture of state of the art best practices software architecture and design. the design details are in cairn but after you are done or by time you're done feature documentation should be available. pull it routinely until you see details about feature spec and then you are the only writer and don't need to check.
```

## Normalized purpose

Maintain Cairn as the generic prompt and workflow library, sync portable components via sync script, and organize repo docs into a clean hierarchy.

## Initial thoughts

- Generic stage contracts and tools live in Cairn; target repositories keep local adapters in profiles.
- Documentation is structured into root entry points, doc/ historical records, and planning/ plans.

## Inferred suggestions

- Inference: Update AGENTS.md, DOCUMENTATION_PROTOCOL.md, and doc/README.md to establish the documentation map.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
