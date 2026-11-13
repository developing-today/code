"""Pure metadata tests for the reference PKI graph (no crypto oracle needed)."""

import unittest
from dataclasses import replace

from pki_graph import (
    ALGORITHMS,
    CLASSICAL,
    PARTITIONS,
    PURPOSES,
    ROOT_NOT_AFTER,
    ISSUER_NOT_AFTER,
    build_reference_graph,
    issuer_key_id,
    pq_only_auth_path,
    require_named_signer_source,
)


class ReferenceGraphTests(unittest.TestCase):
    def setUp(self):
        self.graph = build_reference_graph()

    def test_reference_inventory_is_20_keys_and_65_ca_certificates(self):
        self.assertEqual(len(self.graph.keys), 20)
        self.assertEqual(len({key.key_id for key in self.graph.keys}), 20)
        self.assertEqual(len(self.graph.certificates), 65)
        self.assertEqual(len({cert.cert_id for cert in self.graph.certificates}), 65)
        self.assertEqual(
            {key.algorithm for key in self.graph.keys if key.role == "root"},
            set(ALGORITHMS),
        )

    def test_root_inventory_is_four_self_certificates_and_nine_crosses(self):
        self.assertEqual(len(self.graph.certs_for("root-self")), 4)
        root_certs = self.graph.certs_for("root-self") + self.graph.certs_for("root-cross")
        self.assertEqual(len(root_certs), 13)
        self.assertEqual({cert.not_after for cert in root_certs}, {ROOT_NOT_AFTER})
        crosses = self.graph.certs_for("root-cross")
        self.assertEqual(len(crosses), 9)
        self.assertEqual(
            sum(c.signature_algorithm in CLASSICAL for c in crosses), 9
        )
        self.assertEqual(
            {(c.signer_key, c.subject_key) for c in crosses
             if c.signer_key.rsplit("/", 1)[-1] in CLASSICAL
             and c.subject_key.rsplit("/", 1)[-1] in CLASSICAL},
            {(f"root/{signer}", f"root/{subject}")
             for signer in CLASSICAL for subject in CLASSICAL if signer != subject},
        )
        self.assertEqual(
            {(c.signer_key, c.subject_key) for c in crosses
             if c.subject_key == "root/ml-dsa-44"},
            {(f"root/{signer}", "root/ml-dsa-44") for signer in CLASSICAL},
        )
        self.assertFalse(any(
            c.signature_algorithm == "ml-dsa-44"
            and c.subject_key.endswith(("rsa-4096", "p256", "ed25519"))
            for c in crosses
        ))

    def test_each_purpose_partition_has_13_issuer_certificates(self):
        issuer_certs = self.graph.certs_for("issuer-cert")
        self.assertEqual(len(issuer_certs), 52)
        self.assertEqual({cert.not_after for cert in issuer_certs}, {ISSUER_NOT_AFTER})
        for purpose in PURPOSES:
            for partition in PARTITIONS:
                group = [
                    c for c in issuer_certs
                    if c.purpose == purpose and c.partition == partition
                ]
                self.assertEqual(len(group), 13)
                for algorithm in ALGORITHMS:
                    subject = issuer_key_id(purpose, algorithm, partition)
                    signer_algorithms = {
                        c.signature_algorithm for c in group if c.subject_key == subject
                    }
                    expected = set(CLASSICAL) if algorithm in CLASSICAL else set(ALGORITHMS)
                    self.assertEqual(signer_algorithms, expected)

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

    def test_full_certificate_edge_set_matches_reference_matrix(self):
        def edge(cert):
            return (
                cert.subject_key,
                cert.signer_key,
                cert.signature_algorithm,
                cert.role,
                cert.purpose,
                cert.partition,
            )

        expected = {
            (f"root/{alg}", f"root/{alg}", alg, "root-self", None, None)
            for alg in ALGORITHMS
        }
        expected.update(
            (f"root/{subject}", f"root/{signer}", signer, "root-cross", None, None)
            for signer in CLASSICAL
            for subject in set(ALGORITHMS)
            if subject != signer and subject in CLASSICAL | {"ml-dsa-44"}
        )
        for purpose in PURPOSES:
            for algorithm in ALGORITHMS:
                eligible = set(CLASSICAL) if algorithm in CLASSICAL else set(ALGORITHMS)
                for partition in PARTITIONS:
                    subject = issuer_key_id(purpose, algorithm, partition)
                    for signer in eligible:
                        expected.add((
                            subject,
                            f"root/{signer}",
                            signer,
                            "issuer-cert",
                            purpose,
                            partition,
                        ))

        actual = {edge(cert) for cert in self.graph.certificates}
        self.assertEqual(actual, expected)

        # A mutation can preserve total counts yet violate the exact edge oracle.
        mutated = list(self.graph.certificates)
        index = next(i for i, cert in enumerate(mutated) if cert.role == "issuer-cert")
        mutated[index] = replace(mutated[index], signer_key="root/ml-dsa-44")
        self.assertNotEqual({edge(cert) for cert in mutated}, expected)

    def test_each_certificate_references_known_keys_and_signer_algorithm(self):
        keys = {key.key_id: key for key in self.graph.keys}
        for cert in self.graph.certificates:
            self.assertIn(cert.subject_key, keys)
            self.assertIn(cert.signer_key, keys)
            self.assertEqual(cert.signature_algorithm, keys[cert.signer_key].algorithm)

    def test_pq_auth_metadata_requires_anchor_leaf_and_every_edge(self):
        self.assertTrue(pq_only_auth_path(
            "ml-dsa-44", "ml-dsa-44", ("ml-dsa-44", "ml-dsa-44")
        ))
        self.assertFalse(pq_only_auth_path(
            "ml-dsa-44", "ml-dsa-44", ()
        ))
        self.assertFalse(pq_only_auth_path(
            "rsa-4096", "ml-dsa-44", ("ml-dsa-44", "ml-dsa-44")
        ))
        self.assertFalse(pq_only_auth_path(
            "ml-dsa-44", "ed25519", ("ml-dsa-44", "ml-dsa-44")
        ))
        self.assertFalse(pq_only_auth_path(
            "ml-dsa-44", "ml-dsa-44", ("ed25519", "ml-dsa-44")
        ))
        self.assertFalse(pq_only_auth_path(
            "ml-dsa-44", "ml-dsa-44", ("ml-dsa-44", "rsa-4096")
        ))
        self.assertFalse(pq_only_auth_path(
            "ml-dsa-44", "unknown", ("ml-dsa-44",)
        ))
        self.assertFalse(pq_only_auth_path(
            "ml-dsa-44", "ml-dsa-44", ("unknown",)
        ))

    def test_named_source_presence_guard_is_not_authorization(self):
        for source in (None, "", "  "):
            with self.assertRaises(ValueError):
                require_named_signer_source(source)
        self.assertEqual(
            require_named_signer_source("approved:offline-transaction"),
            "approved:offline-transaction",
        )


if __name__ == "__main__":
    unittest.main()
