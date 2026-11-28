#!/usr/bin/env python3
"""Lane prompts call `fleet agent next` instead of hand-written mailbox
and claim steps (issue #12236, item 1).

Oracle: the in-repo lane-run docs prescribe `fleet agent next` as the
first step of a run, with the hand-written `fleet bridge hello` /
`inbox` / queue-claim steps kept only as the fallback for seats whose
binary predates `fleet agent`. A doc that teaches the manual steps first
is a regression: lanes claim from the queue only and never reach the
push, shortlist or priority rungs.
"""

import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHANNEL = ROOT / "doc" / "agents" / "fleet-command-channel.md"
ONBOARDING = ROOT / "doc" / "agents" / "onboarding-an-agent.md"
NEXT_WORK = ROOT / ".agents" / "skills" / "next-work" / "SKILL.md"


def section(text, heading):
    start = text.index(heading)
    rest = text[start + len(heading) :]
    end = rest.find("\n## ")
    return rest if end == -1 else rest[:end]


class LanePromptsUseAgentNextTests(unittest.TestCase):
    def test_command_channel_run_starts_with_agent_next(self):
        text = CHANNEL.read_text()
        run = section(text, "## Every lane run")
        first_next = run.index("fleet agent next")
        first_manual = min(
            run.index("fleet bridge hello"),
            run.index("fleet bridge inbox"),
        )
        self.assertLess(
            first_next,
            first_manual,
            "the lane run must reach `fleet agent next` before any "
            "hand-written mailbox step",
        )

    def test_command_channel_manual_steps_are_fallback_only(self):
        text = CHANNEL.read_text()
        run = section(text, "## Every lane run")
        self.assertIn("By hand, only when `fleet agent next` cannot run", run)

    def test_onboarding_prescribes_agent_next(self):
        self.assertIn("fleet agent next", ONBOARDING.read_text())

    def test_next_work_skill_prescribes_agent_next(self):
        self.assertIn("fleet agent next", NEXT_WORK.read_text())


if __name__ == "__main__":
    unittest.main()
