# Electrician brief: basement cluster circuits

One-page brief for quoting and scheduling. Program: basement cluster
(epic #1083, roadmap [`basement-cluster.md`](basement-cluster.md)).
This brief is the attachment for engaging an electrician (#1104).

## The job

Install **4 dedicated 20A circuits**, each on its **own breaker**, from the
electrical panel across the open, unfinished basement — along the ceiling,
past ~3 simple walls — to the server room, ending in receptacles at the
rack location. Direct dedicated runs are what we want; an earlier idea of
dropping off a UPS breaker is **not** wanted. Existing cable paths may be
reused where they are code-compliant and save work — electrician's call on
site.

## Load math (why 4 circuits)

Planning assumption, to be replaced by measured values later: **600 W
sustained draw per machine** (Threadripper workstation under load,
including its drives). A 20A / 120V circuit derated to 80% for continuous
load gives **1920 W usable** per circuit.

| Machines | Load at 600 W each | Circuits needed (load / 1920 W, rounded up) |
| -------- | ------------------ | ------------------------------------------- |
| 3        | 1800 W             | 1                                           |
| 5        | 3000 W             | 2                                           |
| 12       | 7200 W             | 4                                           |

The first 3–5 machines need 2 circuits; the full ~12-machine fleet needs
4 (4 × 1920 W = 7680 W usable ≥ 7200 W). Installing all 4 now avoids a
second visit when the fleet grows; the first machines spread across 2
circuits and the rest stay spare until needed.

## Spec per circuit

- 20A single-pole breaker, dedicated — no shared loads on these circuits.
- 12 AWG copper run, panel to server room, along the basement ceiling.
- Duplex receptacles at the rack location in the server room (exact
  placement and height confirmed on the site visit).
- Each circuit labeled at the panel and at the receptacle
  ("CLUSTER 1" … "CLUSTER 4").

## Open items — for the site visit, not guessed here

- Panel make, service capacity, and number of free breaker spaces.
- Exact run lengths and the final ceiling route past the ~3 walls.
- Whether the existing cable paths are reusable as-is.
- Receptacle count, placement, and height at the rack.
- Local code and permit requirements for this work (including any
  GFCI/AFCI requirements that apply to the location).

## What we need back

A quote for the 4-circuit install as specced above, noting any open item
that changes the price, and the earliest scheduling window.
