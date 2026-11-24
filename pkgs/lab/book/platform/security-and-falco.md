# Security: Falco runtime detection

The cluster this book plans will run agent workloads — software
that shells out, reads credential files and talks to the network
as its normal mode of work. That makes the estate's runtime-
security question sharper than usual: the tripwire has to tell a
misbehaving or compromised workload from an agent doing its job.
**The design's tripwire layer is Falco (Drew, 2026-11-23).**
Status: **not deployed — candidate adopted as the design's
runtime-detection layer; there is no cluster to deploy it on.**

## What Falco is in this design

Falco is a runtime-detection tool: it watches system calls on a
host — via eBPF probes on modern kernels — and matches the
behaviour it sees against a rule set. In this estate's design
its job is **detection, not prevention**: Falco raises the
alarm; it does not block the call, kill the container or enforce
policy. Enforcement, where the design wants it, belongs to
other layers; Falco is the layer that notices.

The behaviours it is adopted to watch for, named plainly:

- **A shell spawned in a container** that has no business
  running one — the classic first move after a workload is
  compromised.
- **Reads of credential files** — a process opening secret or
  credential material outside its expected paths.
- **Unexpected egress** — a workload opening network
  connections its role does not explain, in an estate whose
  egress strategy is deliberately narrow (see
  [The cluster programme](../cluster/the-cluster-programme.md)).

For an estate running agent workloads, those three are the
tripwire's centre: each is something an agent *can* do
legitimately in the right context and an intruder *must* do in
almost any context, which is exactly what makes the tuning cost
below real rather than hypothetical.

## The tuning cost, stated honestly

Falco's cost is not the probe; it is the rule set. A stock rule
set on an estate full of agents — which spawn shells, read
credentials and make network calls as their day job — will
produce a false-positive load that trains its operators to
ignore it, and an ignored tripwire is worse than none. The
design therefore carries, as known and accepted costs:

- **Per-workload rule tuning:** the expected-behaviour profile
  differs per workload, and writing it is real work that lands
  with each workload, not once for the cluster.
- **An alert-triage habit:** detections need a named place to
  go and a human or agent routine that reads them; Falco
  without a consumer is log volume, not security. The natural
  consumer is the estate's automation layer — see
  [Automation and automated scaling](automation-and-scaling.md) —
  but no wiring exists and none is claimed.
- **Kernel/eBPF fit on Talos:** Falco's eBPF delivery on an
  immutable, declarative OS like Talos is a deployment question
  to prove at install time, not a property this book asserts.

## Where it sits in the programme

- Cluster home: the cluster programme's platform-services and
  observability work (epics
  [#1088](https://github.com/developing-today/code/issues/1088)
  and
  [#1090](https://github.com/developing-today/code/issues/1090)) —
  see [The cluster programme](../cluster/the-cluster-programme.md),
  which cross-links this chapter.
- It watches workloads; it does not replace the estate's other
  boundaries (egress design, secrets handling, declarative
  rebuild-from-config), and this chapter claims no coverage
  beyond runtime behaviour on cluster nodes.

## Current state

**Not deployed.** No Falco configuration, rule set or alert
route exists anywhere in the estate today. What exists is this
decision of record (Drew, 2026-11-23): Falco is the adopted
candidate for the runtime-detection layer, to be installed and
tuned when a cluster exists to run it on.
