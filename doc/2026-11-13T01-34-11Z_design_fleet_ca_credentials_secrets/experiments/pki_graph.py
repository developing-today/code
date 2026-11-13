"""Synthetic Fleet v0.3 CA inventory and policy graph.

This module models names, keys, and certificate-signing edges only.  It does
not create keys, encode X.509, perform cryptography, read stores, or publish
artifacts.  It is an executable arithmetic/policy oracle for the reference
inventory in spec §§3.1–3.2 and 24.2.
"""

from __future__ import annotations

from dataclasses import dataclass
from itertools import product


ALGORITHMS = ("rsa-4096", "p256", "ed25519", "ml-dsa-44")
CLASSICAL = frozenset(("rsa-4096", "p256", "ed25519"))
PURPOSES = ("base", "internal-web")
PARTITIONS = ("A", "B")


@dataclass(frozen=True)
class Key:
    key_id: str
    algorithm: str
    role: str
    purpose: str | None = None
    partition: str | None = None


@dataclass(frozen=True)
class Certificate:
    cert_id: str
    subject_key: str
    signer_key: str
    signature_algorithm: str
    role: str
    purpose: str | None = None
    partition: str | None = None


@dataclass(frozen=True)
class ReferenceGraph:
    keys: tuple[Key, ...]
    certificates: tuple[Certificate, ...]

    def certs_for(self, role: str) -> tuple[Certificate, ...]:
        return tuple(c for c in self.certificates if c.role == role)


def root_key_id(algorithm: str) -> str:
    return f"root/{algorithm}"


def issuer_key_id(purpose: str, algorithm: str, partition: str) -> str:
    return f"issuer/{purpose}/{algorithm}/{partition}"


def build_reference_graph() -> ReferenceGraph:
    """Return the exact reference graph as inert, deterministic metadata."""
    keys: list[Key] = [
        Key(root_key_id(alg), alg, "root") for alg in ALGORITHMS
    ]
    keys.extend(
        Key(issuer_key_id(purpose, alg, partition), alg, "issuer", purpose, partition)
        for purpose, alg, partition in product(PURPOSES, ALGORITHMS, PARTITIONS)
    )

    certs: list[Certificate] = []
    for alg in ALGORITHMS:
        key = root_key_id(alg)
        certs.append(Certificate(f"root-self/{alg}", key, key, alg, "root-self"))

    # Every directed classical-to-classical edge, plus classical-to-PQ.
    for signer_alg in ALGORITHMS:
        for subject_alg in ALGORITHMS:
            if signer_alg == subject_alg:
                continue
            if signer_alg in CLASSICAL and subject_alg in CLASSICAL | {"ml-dsa-44"}:
                signer = root_key_id(signer_alg)
                subject = root_key_id(subject_alg)
                certs.append(Certificate(
                    f"root-cross/{subject_alg}/by-{signer_alg}",
                    subject,
                    signer,
                    signer_alg,
                    "root-cross",
                ))

    # Direct issuer signing matrix: classical roots sign every classical
    # issuer and the ML-DSA issuer; the ML-DSA root signs ML-DSA only.
    for purpose, alg, partition in product(PURPOSES, ALGORITHMS, PARTITIONS):
        subject = issuer_key_id(purpose, alg, partition)
        eligible = CLASSICAL if alg in CLASSICAL else CLASSICAL | {"ml-dsa-44"}
        for signer_alg in sorted(eligible):
            signer = root_key_id(signer_alg)
            certs.append(Certificate(
                f"issuer-cert/{purpose}/{alg}/{partition}/by-{signer_alg}",
                subject,
                signer,
                signer_alg,
                "issuer-cert",
                purpose,
                partition,
            ))

    return ReferenceGraph(tuple(keys), tuple(certs))


def pq_only_path(anchor_algorithm: str, signature_algorithms: tuple[str, ...]) -> bool:
    """Whether an asserted certification path is ML-DSA-only.

    The caller supplies the anchor algorithm and each certificate signature
    algorithm in leaf-to-anchor order.  This checks policy metadata, not a
    certificate signature or path's validity.
    """
    return anchor_algorithm == "ml-dsa-44" and all(
        algorithm == "ml-dsa-44" for algorithm in signature_algorithms
    )


def require_explicit_signer_source(signer_source: str | None) -> str:
    """Reject ambient/default signer discovery; this is a policy-only guard."""
    if signer_source is None or not signer_source.strip():
        raise ValueError("signing requires an explicit authorized signer source")
    return signer_source


if __name__ == "__main__":
    graph = build_reference_graph()
    print(
        f"synthetic keys={len(graph.keys)} certs={len(graph.certificates)} "
        f"root-cross={len(graph.certs_for('root-cross'))} "
        f"issuer-certs={len(graph.certs_for('issuer-cert'))}"
    )
