# Issue #6053 Analysis: License Capture at Ingest

**Task:** 10419  
**Issue:** #6053 - design(atlas): license capture at ingest — per-repo license + D.6 applicability flag in provenance  
**Parent Issue:** #6024 (licensing gate for external corpus)  
**Provenance Substrate:** #3125  

---

## Executive Summary

**Status:** ✅ Issue verified as OPEN with no blocking PRs  
**Worktree:** `/home/user/git/fleet/10419` (branch: `queue/task-10419-issue-6053-license-capture`)  
**Current Main:** `8ad60f1ef2de3ec3d14bc2374939b5bd6849486b`  

**Finding:** This design requirement is **NOT YET IMPLEMENTED**. No existing code captures provenance fields for external repository ingestion.

---

## Requirements from Issue #6053

For every external repo ingested, record in provenance:

1. ✅ **Detected license** (SPDX id where detectable)
   - Source: licensing family's tooling (#5979)
   
2. ✅ **D.6 inbound=outbound applicability flag**
   - Record the flag and license version
   - Note: "repo carries a license notice → comments arguably under the same terms"
   - Do NOT draw conclusions, just record the data
   
3. ✅ **Zone assignment** (sibling policy)
   
4. ✅ **Ingest date**

**Purpose:** Enable policy re-evaluation without re-crawling the web. Later policy changes can re-evaluate rows, not re-crawl.

---

## Current Codebase State

### Provenance System
- ❌ **No provenance field system exists** for external repository ingestion
- ✅ Provenance concept exists in other contexts (capability matrices, test provenance, etc.) but not for Atlas ingestion
- ✅ `pm-stack/aggregator/scripts/` has provenance tracking for feeds and sources, but not for external repo licensing

### License Detection
- ❌ **No license detection mechanism** for external repositories
- ✅ SPDX identifiers exist for this project's own files (MIT OR Apache-2.0)
- ❌ No code references #5979 (licensing family's tooling detector source)
- ❌ No code handles external repo license detection

### Atlas Ingestion
- ⚠️ **Atlas references found are dashboard/backup related** (~/atlas/, atlas-backup.sh)
- ❌ No evidence of Atlas ingestion system for external repos
- ❌ No code handles external repository ingestion workflow

### Parent Issue Context (#6024)
- Parent issue mentions: "licensing gate for the external corpus"
- Research source: prior-art doc from 2026-11-04
- Follow-up 32 (Cluster 6)
- This issue (#6053) is a sibling decision under #6024

---

## Implementation Plan

### Phase 1: Provenance Field Design

**Files to create/modify:**

1. **`doc/2026-11-05_6053-provenance-design.md`** - Detailed design document
   - Field definitions
   - JSON schema for provenance records
   - Detection source specifications
   - Re-evaluation query examples

2. **`planning/designs/6053-provenance-fields.md`** - ADR-style design record

### Phase 2: License Detection Integration

**Requirements:**
- Integrate with #5979 (licensing family's tooling)
- Detect SPDX license IDs from external repos
- Handle cases where license is not detectable

**Implementation:**
- Create `src/atlas/license-detection.ts` (or similar)
- Implement license detection function
- Add caching layer to avoid repeated detection

### Phase 3: Provenance Capture at Ingest

**Where ingestion happens:** (to be identified)
- Add provenance capture hooks in ingestion pipeline
- Record: license, D.6 flag, zone assignment, ingest date
- Store in database (likely SQLite/Postgres)

### Phase 4: Policy Re-evaluation Query

**Design:**
- Create SQL query/function for policy layer to re-evaluate provenance rows
- Support filtering by license, D.6 flag, zone, ingest date
- Document usage examples

---

## Blockers and Dependencies

### Dependencies
1. ✅ Issue #6024 (parent) - licensing gate decision
2. ❌ Issue #5979 - licensing family's tooling detector
3. ❌ Atlas ingestion system (if not already implemented elsewhere)

### Questions for Resolution
1. Where does external repo ingestion currently happen? (Need to find ingestion code)
2. What database stores this data? (SQLite, Postgres, etc.)
3. What is the "zone assignment" policy? (Need to find zone policy doc)
4. What format should provenance records take? (JSON schema needed)

---

## Risk Assessment

| Risk | Level | Mitigation |
|------|-------|------------|
| License detection unreliable | Medium | Use multiple detection methods, fallback to "unknown" |
| Policy changes break re-evaluation | Low | Design flexible query interface |
| Performance impact on ingestion | Medium | Implement caching, async processing |
| Missing parent issue context | Low | Coordinate with #6024 owner |

---

## Next Steps

### Immediate (This Task - Triage Complete)
- ✅ Issue verified as OPEN
- ✅ No blocking PRs found
- ✅ Worktree prepared
- ✅ Agent note posted
- ✅ Labels updated (queue/claimed, removed parked/status labels)

### Short-term
1. **Find ingestion code:** Search for where external repos are ingested
2. **Design provenance schema:** Create JSON schema for provenance records
3. **Implement license detection:** Integrate with #5979
4. **Create ADR:** Document the design decision

### Medium-term
1. **Implement capture hooks:** Add provenance recording at ingest time
2. **Implement re-evaluation query:** Create policy layer query
3. **Add tests:** Test license detection, provenance capture, re-evaluation
4. **Document:** Update README and design docs

### Long-term
1. **Deploy to staging:** Test with real external repos
2. **Monitor:** Track license detection accuracy
3. **Refine:** Adjust based on actual usage patterns

---

## Resources

- **Issue:** https://github.com/developing-today/code/issues/6053
- **Parent Issue:** https://github.com/developing-today/code/issues/6024
- **Provenance Substrate:** https://github.com/developing-today/code/issues/3125
- **Licensing Tooling:** Issue #5979
- **Research Source:** `~/workspace/scratch/atlas-ingestion-brief-2026-11-04/prior-art-research.md`

---

## Evidence

### Code Search Results
- **provenance**: Found in capability matrices, test provenance, but NOT for external repo ingestion
- **license detection**: Only for this project's own SPDX headers
- **atlas ingestion**: References are dashboard/backup related, not ingestion
- **external repos**: No ingestion code found

### GitHub API Check
```json
{
  "issue": 6053,
  "state": "OPEN",
  "title": "design(atlas): license capture at ingest — per-repo license + D.6 applicability flag in provenance",
  "labels": ["area/infra", "kind/design"],
  "prs": []
}
```

---

## Conclusion

**Issue #6053 requires a NEW implementation.** No existing code satisfies the requirements. The design needs to be created from scratch, including:

1. Provenance field schema
2. License detection integration (#5979)
3. Ingestion pipeline modifications
4. Policy re-evaluation query

**Recommendation:** Create implementation plan issue and proceed with design phase.

---

**Analysis Date:** 2026-11-05  
**Analyst:** OpenChamber scheduled lane (lane ID: 7b47536b-c162-444c-be8a-3a9538cc4cb5)  
**Task:** 10419 - Triage Complete ✅
