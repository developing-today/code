"""Contract tests for staged #6651 alignment requests.

Oracle: the task's recorded contract — a qualifying request has a tracked
ID/owner/scope/evidence and receives ACK (or BLOCKED) followed by RESULT
in the originating thread. The fake sender is deliberately offline.
"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import handler


CHANNEL = "C0CBYQR65LL"
OWNER = "U0CBX163G8Z"
BOT = "U0CBTJGQQSZ"


class FakeSender:
    def __init__(self):
        self.messages = []

    def send(self, channel, thread_ts, text, idempotency_key):
        self.messages.append((channel, thread_ts, text, idempotency_key))
        return {"ok": True, "ts": f"reply-{len(self.messages)}"}


def event(ts, text, *, user=OWNER, channel=CHANNEL, thread_ts=None):
    return {
        "type": "message",
        "channel": channel,
        "ts": ts,
        "thread_ts": thread_ts,
        "user": user,
        "text": text,
    }


class AlignmentHandlerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.db = os.path.join(self.temp.name, "alignment.db")
        self.sender = FakeSender()
        self.handler = handler.AlignmentHandler(
            self.db,
            channel_id=CHANNEL,
            owner_user_id=OWNER,
            bot_user_id=BOT,
        )

    def tearDown(self):
        self.handler.close()
        self.temp.cleanup()

    def test_staged_request_ack_then_result_in_originating_thread(self):
        source = event(
            "100.001",
            "<@U0CBTJGQQSZ>\n"
            "ID: align-17\nOwner: @drew\n"
            "Scope: review the staged parser\n"
            "Evidence: https://example.invalid/pr/17",
        )

        request_id = self.handler.handle_event(source, self.sender)
        self.assertEqual(request_id, "align-17")
        item = self.handler.get_request(request_id)
        self.assertEqual(item["owner"], "@drew")
        self.assertEqual(item["scope"], "review the staged parser")
        self.assertEqual(item["evidence"], "https://example.invalid/pr/17")
        self.assertEqual(item["state"], "acknowledged")

        self.handler.complete_request(
            request_id,
            result="The staged parser matches the expected contract.",
            evidence=["https://example.invalid/run/17"],
            sender=self.sender,
        )

        self.assertEqual(len(self.sender.messages), 2)
        ack, result = self.sender.messages
        self.assertEqual((ack[0], ack[1]), (CHANNEL, "100.001"))
        self.assertTrue(ack[2].startswith("ACK align-17"), ack[2])
        self.assertEqual((result[0], result[1]), (CHANNEL, "100.001"))
        self.assertTrue(result[2].startswith("RESULT align-17"), result[2])
        self.assertEqual(self.handler.get_request(request_id)["state"], "complete")

    def test_missing_scope_is_blocked_and_names_the_missing_field(self):
        request_id = self.handler.handle_event(
            event(
                "101.001",
                "<@U0CBTJGQQSZ>\nID: align-18\nOwner: @drew\n"
                "Evidence: https://example.invalid/pr/18",
            ),
            self.sender,
        )

        item = self.handler.get_request(request_id)
        self.assertEqual(item["state"], "blocked")
        self.assertIn("scope", item["missing"])
        self.assertTrue(self.sender.messages[0][2].startswith("BLOCKED align-18"))

    def test_reply_in_participating_thread_is_a_trigger(self):
        first = event(
            "102.001",
            "<@U0CBTJGQQSZ>\nID: align-19\nOwner: @drew\n"
            "Scope: inspect reply\nEvidence: https://example.invalid/19",
        )
        request_id = self.handler.handle_event(first, self.sender)
        reply = event(
            "102.002",
            "ID: align-20\nOwner: @drew\nScope: follow-up\n"
            "Evidence: https://example.invalid/20",
            user="U0CBTJGQQSZ",
            thread_ts="102.001",
        )

        reply_id = self.handler.handle_event(reply, self.sender)
        self.assertEqual(reply_id, "align-20")
        self.assertEqual(self.handler.get_request(reply_id)["thread_ts"], "102.001")
        self.assertTrue(self.sender.messages[1][2].startswith("ACK align-20"))

    def test_unmentioned_owner_message_outside_participating_thread_is_ignored(self):
        result = self.handler.handle_event(
            event("103.001", "ordinary channel message"), self.sender
        )
        self.assertIsNone(result)
        self.assertEqual(self.sender.messages, [])

    def test_duplicate_slack_event_does_not_send_twice(self):
        source = event(
            "104.001",
            "<@U0CBTJGQQSZ>\nID: align-21\nOwner: @drew\n"
            "Scope: dedupe\nEvidence: https://example.invalid/21",
        )
        self.assertEqual(self.handler.handle_event(source, self.sender), "align-21")
        self.assertIsNone(self.handler.handle_event(source, self.sender))
        self.assertEqual(len(self.sender.messages), 1)


if __name__ == "__main__":
    unittest.main()
