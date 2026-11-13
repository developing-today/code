"""Pure metadata tests for the reference PKI graph (no crypto oracle needed)."""

import unittest

from pki_graph import (
    ALGORITHMS,
    CLASSICAL,
    PARTITIONS,
    PURPOSES,
    build_reference_graph,
    issuer_key_id,
    pq_only_path,
    require_explicit_signer_source,
)


class ReferenceGraphTests(unittest.TestCase):
    def setUp(self):
        self.graph = build_reference_graph()

    def test_reference_inventory_is_20_keys_and_65_ca_certificates(self):
        self.assertEqual(len(self.graph.keys), 20)
        self.assertEqual(len({key.key_id for key in self.graph.keys}), 20)
        self.assertEqual(len(self.graph.certificates), 65)
        self.assertEqual(len({cert.cert_id for cert in self.graph.certificates}), 65)

    def test_root_inventory_is_four_self_certificates_and_nine_crosses(self):
        self.assertEqual(len(self.graph.certs_for("root-self")), 4)
        crosses = self.graph.certs_for("root-cross")
        self.assertEqual(len(crosses), 9)
        self.assertEqual(
            sum(c.signature_algorithm in CLASSICAL for c in crosses), 9
        )
        self.assertFalse(any(
            c.signature_algorithm == "ml-dsa-44"
            and c.subject_key.endswith(("rsa-4096", "p256", "ed25519"))
            for c in crosses
        ))

    def test_each_purpose_partition_has_13_issuer_certificates(self):
        issuer_certs = self.graph.certs_for("issuer-cert")
        self.assertEqual(len(issuer_certs), 52)
        for purpose in PURPOSES:
            for partition in PARTITIONS:
                group = [
                    c for c in issuer_certs
                    if c.purpose == purpose and c.partition == partition
                ]
                self.assertEqual(len(group), 13)

    def test_partitions_are_distinct_keys(self):
        for purpose in PURPOSES:
            for algorithm in ALGORITHMS:
                a = issuer_key_id(purpose, algorithm, "A")
                b = issuer_key_id(purpose, algorithm, "B")
                self.assertNotEqual(a, b)
                self.assertIn(a, {key.key_id for key in self.graph.keys})
                self.assertIn(b, {key.key_id for key in self.graph.keys})

    def test_ml_dsa_root_never_signs_classical_issuer(self):
        for cert in self.graph.certs_for("issuer-cert"):
            if cert.signer_key == "root/ml-dsa-44":
                self.assertIn(cert.subject_key.split("/")[2], {"ml-dsa-44"})

    def test_pq_only_policy_requires_ml_dsa_anchor_and_every_edge(self):
        self.assertTrue(pq_only_path("ml-dsa-44", ("ml-dsa-44", "ml-dsa-44")))
        self.assertFalse(pq_only_path("rsa-4096", ("ml-dsa-44", "ml-dsa-44")))
        self.assertFalse(pq_only_path("ml-dsa-44", ("ed25519", "ml-dsa-44")))
        self.assertFalse(pq_only_path("ml-dsa-44", ("ml-dsa-44", "rsa-4096")))

    def test_signing_rejects_ambient_or_missing_source(self):
        for source in (None, "", "  "):
            with self.assertRaises(ValueError):
                require_explicit_signer_source(source)
        self.assertEqual(
            require_explicit_signer_source("approved:offline-transaction"),
            "approved:offline-transaction",
        )


if __name__ == "__main__":
    unittest.main()
