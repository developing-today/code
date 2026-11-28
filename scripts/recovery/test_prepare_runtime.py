import unittest, random, json
from prepare_runtime import text_array, expr


class RuntimeTests(unittest.TestCase):
    def test_array_nulls_escapes(self):
        self.assertEqual(
            text_array(r'{NULL,"NULL","a,b","a\"b","c\\d",""}'),
            [None, "NULL", "a,b", 'a"b', r"c\d", ""],
        )
        self.assertEqual(text_array("{}"), [])

    def test_array_property(self):
        r = random.Random(31)
        for _ in range(1000):
            rows = [
                "".join(r.choice('AB,{}\\"á\t') for _ in range(r.randrange(20)))
                for _ in range(r.randrange(10))
            ]
            encoded = (
                "{"
                + ",".join(
                    '"' + x.replace("\\", "\\\\").replace('"', '\\"') + '"'
                    for x in rows
                )
                + "}"
            )
            self.assertEqual(text_array(encoded), rows)

    def test_array_failclosed(self):
        for s in ["{{a}}", '{"oops}', "{a,}", "[1:2]={a,b}"]:
            with self.assertRaises(ValueError):
                text_array(s)


if __name__ == "__main__":
    unittest.main()
