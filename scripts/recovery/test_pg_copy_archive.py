import unittest, random, tempfile, sqlite3, io
from pathlib import Path
from pg_copy_archive import decode, convert


class ArchiveTests(unittest.TestCase):
    def test_escapes(self):
        self.assertIsNone(decode(r"\N"))
        self.assertEqual(decode(r"\\N"), r"\N")
        self.assertEqual(decode(r"a\tb\nc\\d"), "a\tb\nc\\d")
        self.assertEqual(decode(r"\101\x42"), "AB")

    def test_roundtrip_property(self):
        r = random.Random(41)
        for _ in range(1000):
            x = "".join(r.choice("ABCá中\t\n\r\\012") for _ in range(r.randrange(80)))
            encoded = (
                x.replace("\\", r"\\")
                .replace("\t", r"\t")
                .replace("\n", r"\n")
                .replace("\r", r"\r")
            )
            self.assertEqual(decode(encoded), x)

    def test_archive_oracle(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            s = p / "s"
            s.write_text(
                "COPY public.t (id, value) FROM stdin;\n9223372036854775809\t\\N\n1\t\\\\N\n\\.\n"
            )
            m = convert(s, p / "d", p / "r")
            self.assertEqual(m["total_rows"], 2)
            with sqlite3.connect(p / "d") as db:
                self.assertEqual(
                    db.execute('select * from "public.t"').fetchall(),
                    [("9223372036854775809", None), ("1", r"\N")],
                )
            with self.assertRaises(ValueError):
                convert(s, p / "d", p / "r")

    def test_empty_ddl(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            s = p / "s"
            s.write_text(
                "CREATE TABLE realtime.messages (\n    topic text NOT NULL,\n    id uuid DEFAULT gen_random_uuid() NOT NULL\n)\nPARTITION BY RANGE (id);\n"
            )
            m = convert(s, p / "d", p / "r")
            self.assertEqual(
                m["tables"]["realtime.messages"]["columns"], ["topic", "id"]
            )
            self.assertEqual(m["total_rows"], 0)

    def test_quoted_identifier_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            s = p / "s"
            s.write_text('COPY public.t ("comma,name") FROM stdin;\n1\n\\.\n')
            with self.assertRaises(ValueError):
                convert(s, p / "d", p / "r")

    def test_source_change_rejected(self):
        class Changing:
            def open(self, mode="r", **kw):
                text = "COPY public.t (id) FROM stdin;\n1\n\\.\n"
                return (
                    io.BytesIO(text.encode())
                    if mode == "rb"
                    else io.StringIO(text + "-- changed\n")
                )

        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            with self.assertRaisesRegex(ValueError, "source changed"):
                convert(Changing(), p / "d", p / "r")

    def test_unsupported_or_truncated_ddl_rejected(self):
        for text in [
            'CREATE TABLE public.t (\n    "a""b" text\n);\n',
            'CREATE TABLE public."T" (\n    id text\n);\n',
            "CREATE TABLE public.t (\n    id text\n",
        ]:
            with tempfile.TemporaryDirectory() as d:
                p = Path(d)
                s = p / "s"
                s.write_text(text)
                with self.assertRaises(ValueError):
                    convert(s, p / "d", p / "r")

    def test_truncated_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            s = p / "s"
            s.write_text("COPY public.t (id) FROM stdin;\n1\n")
            with self.assertRaises(ValueError):
                convert(s, p / "d", p / "r")


if __name__ == "__main__":
    unittest.main()
