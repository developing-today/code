# Issue #6053 Implementation Summary: License Capture at Ingest

**Task:** 10419  
**Issue:** #6053 - design(atlas): license capture at ingest — per-repo license + D.6 applicability flag in provenance  
**Parent Issue:** #6024 (licensing gate for external corpus)  
**Status:** Analysis Complete - Implementation Required

---

## Executive Summary

**Current State:** ❌ NOT IMPLEMENTED - No existing code captures provenance fields for external repository ingestion.

**Worktree:** `/home/user/git/fleet/10419` (branch: `queue/task-10419-issue-6053-license-capture`)

**Key Finding:** This is a **new implementation requirement** with no existing code to build upon. All provenance, license detection, and ingestion pipeline components need to be created from scratch.

---

## Requirements Analysis

### From Issue #6053

For every external repo ingested, record in provenance:

1. **Detected license** (SPDX id where detectable)
   - Source: licensing family's tooling (#5979)
   - Must integrate with existing license detection infrastructure

2. **D.6 inbound=outbound applicability flag**
   - Boolean flag indicating if repo carries a license notice → comments arguably under same terms
   - Must record the flag value and license version reference
   - **Important:** Do NOT draw conclusions, just record the data

3. **Zone assignment**
   - Sibling policy decision (see dependencies)
   - Must be recorded in provenance

4. **Ingest date**
   - Timestamp of when ingestion occurred
   - Must be recorded in provenance

### Purpose
Enable policy re-evaluation **without re-crawling the web**. Later policy changes can re-evaluate rows, not re-crawl entire repositories.

---

## Current Codebase State

### 1. Provenance System ✅ (Partial)

**Existing Provenance Usage:**
- ✅ Capability provenance matrices in `pm-stack/aggregator/` (for feed sources, not external repos)
- ✅ Test provenance tracking in `pm-stack/aggregator/scripts/check-provenance-matrix.ts`
- ❌ **No provenance field system exists for external repository ingestion**
- ✅ Provenance concept exists in other contexts but not for Atlas ingestion

**Key Files:**
- `/home/user/git/code/pm-stack/aggregator/scripts/check-provenance-matrix.ts` - Shows provenance matrix pattern but for feeds, not external repos
- `/home/user/git/code/doc/2026-10-16T09-00-00Z_feature_world_module_provenance/` - Module provenance documentation (different context)

### 2. License Detection ❌

**Current State:**
- ❌ **No license detection mechanism for external repositories**
- ✅ SPDX identifiers exist for this project's own files (`LICENSE-MIT`, `LICENSE-APACHE-2.0`)
- ❌ No code references #5979 (licensing family's tooling detector source)
- ❌ No code handles external repo license detection
- ❌ No license detection integration points

**SPDX Status:**
- Project uses `MIT OR Apache-2.0` SPDX identifier
- No automated license detection for external repos

### 3. Atlas Ingestion ❌

**Current State:**
- ⚠️ Atlas references found are dashboard/backup related (`~/atlas/`, `atlas-backup.sh`)
- ❌ **No evidence of Atlas ingestion system for external repos**
- ❌ No code handles external repository ingestion workflow
- ❌ No ingestion pipeline for external repositories

**Ingestion Patterns Found:**
- `src/mcpx/internal/logstore/ingest.go` - Log file ingestion (JSONL files)
- `src/id/src/event_store/ingest.rs` - Event ingestion for p2panda system
- These are **not** for external repository ingestion

### 4. External Repository Handling ❌

**No ingestion code found for:**
- External repositories
- Repository cloning/fetching
- License file detection
- Provenance tracking

---

## Implementation Plan

### Phase 1: Provenance Schema Design (1-2 days)

**Objective:** Define the data structure for provenance records

**Deliverables:**
1. `doc/2026-11-05_6053-provenance-schema.md` - Detailed schema documentation
2. `planning/designs/6053-provenance-fields.md` - ADR-style design record

**Schema Requirements:**
```json
{
  "external_repo_provenance": {
    "repo_url": "string (required)",
    "detected_license": {
      "spdx_id": "string (optional)",
      "confidence": "number (0-1)",
      "detected_from": "string (file path or detection method)",
      "version": "string (license version if available)"
    },
    "d6_applicability": {
      "flag": "boolean (required)",
      "rationale": "string (notes about detection)"
    },
    "zone_assignment": "string (required - policy decision)",
    "ingest_date": "ISO8601 timestamp (required)",
    "source_revision": "string (commit hash or tag)",
    "detected_license_files": "array[string] (paths to license files found)"
  }
}
```

**Detection Sources:**
- License files (LICENSE, LICENCE, COPYING)
- Package manifests (package.json, Cargo.toml, pyproject.toml)
- Source headers
- SPDX tags in files
- NOTICE files

### Phase 2: License Detection Integration (3-5 days)

**Objective:** Implement license detection using #5979 tooling

**Deliverables:**
1. `src/atlas/license-detection.ts` (or `src/atlas/license-detection.rs`)
2. Integration with external repository cloning

**Detection Strategy:**
```typescript
interface LicenseDetectionResult {
  spdx_id: string | null;
  confidence: number;
  detected_from: string;
  version: string | null;
  files_checked: string[];
}

function detectLicense(repoPath: string): LicenseDetectionResult {
  // 1. Check common license file names
  // 2. Parse package manifests
  // 3. Check SPDX tags in source files
  // 4. Use #5979 licensing tooling
  // 5. Return best match with confidence score
}
```

**D.6 Applicability Flag Logic:**
```typescript
function calculateD6Flag(repoPath: string, licenseResult: LicenseDetectionResult): {
  flag: boolean;
  rationale: string;
} {
  // D.6: repo carries a license notice → comments arguably under same terms
  // 
  // Flag = true if:
  // - License file exists AND
  // - License is OSI-approved OR widely recognized
  //
  // Flag = false if:
  // - No license detected OR
  // - License is proprietary/non-standard
  
  const hasLicense = licenseResult.spdx_id !== null;
  const isRecognized = isRecognizedLicense(licenseResult.spdx_id);
  
  return {
    flag: hasLicense && isRecognized,
    rationale: hasLicense 
      ? `License ${licenseResult.spdx_id} detected`
      : 'No license detected'
  };
}
```

### Phase 3: External Repository Ingestion Pipeline (5-7 days)

**Objective:** Create ingestion pipeline for external repositories

**Where Ingestion Should Happen:**
Based on codebase patterns, likely location:
- `src/atlas/ingest/` directory
- `src/atlas/external-repo/` directory
- Integration with existing `pm-stack` or `mcpx` systems

**Implementation Steps:**

1. **Repository Cloning Module:**
   ```rust
   // Rust implementation (likely in src/atlas/)
   pub struct ExternalRepoIngest {
     repo_url: String,
     target_path: PathBuf,
     zone: String,
   }
   
   impl ExternalRepoIngest {
     pub async fn clone(&self) -> Result<(), IngestError>;
     pub async fn detect_license(&self) -> LicenseDetectionResult;
     pub async fn calculate_d6_flag(&self, license_result: &LicenseDetectionResult) -> D6FlagResult;
     pub async fn ingest(&self) -> ProvenanceRecord;
   }
   ```

2. **Provenance Capture Hooks:**
   - Add provenance recording at ingest time
   - Store in database (likely SQLite for Atlas, or Postgres)
   - Create migration scripts

3. **Database Schema:**
   ```sql
   CREATE TABLE external_repo_provenance (
     id INTEGER PRIMARY KEY AUTOINCREMENT,
     repo_url TEXT NOT NULL UNIQUE,
     detected_license_spdx TEXT,
     detected_license_confidence REAL,
     detected_license_from TEXT,
     detected_license_version TEXT,
     d6_flag BOOLEAN NOT NULL,
     d6_rationale TEXT,
     zone_assignment TEXT NOT NULL,
     ingest_date TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
     source_revision TEXT,
     detected_license_files TEXT, -- JSON array
     metadata JSON -- Additional metadata
   );
   
   CREATE INDEX idx_external_repo_url ON external_repo_provenance(repo_url);
   CREATE INDEX idx_external_repo_license ON external_repo_provenance(detected_license_spdx);
   CREATE INDEX idx_external_repo_d6_flag ON external_repo_provenance(d6_flag);
   CREATE INDEX idx_external_repo_ingest_date ON external_repo_provenance(ingest_date);
   ```

### Phase 4: Policy Re-evaluation Query (2-3 days)

**Objective:** Create SQL query/function for policy layer re-evaluation

**Deliverables:**
1. SQL function for querying provenance records
2. Documentation with usage examples

**Re-evaluation Query Examples:**

```sql
-- Find all repos with MIT license
SELECT repo_url, ingest_date 
FROM external_repo_provenance 
WHERE detected_license_spdx = 'MIT';

-- Find all repos where D.6 flag is true
SELECT repo_url, d6_flag, zone_assignment 
FROM external_repo_provenance 
WHERE d6_flag = true;

-- Find repos ingested in last 30 days
SELECT repo_url, detected_license_spdx, zone_assignment 
FROM external_repo_provenance 
WHERE ingest_date >= datetime('now', '-30 days');

-- Policy re-evaluation: repos with unknown license
SELECT repo_url, 'REVIEW_REQUIRED' as action 
FROM external_repo_provenance 
WHERE detected_license_spdx IS NULL;
```

**Stored Procedure:**
```sql
CREATE PROCEDURE re_evaluate_provenance(IN license_filter TEXT, IN zone_filter TEXT)
BEGIN
  -- Complex re-evaluation logic based on policy changes
  -- Can update zone assignments, flags, etc.
END;
```

### Phase 5: Testing & Documentation (3-5 days)

**Testing Strategy:**
1. Unit tests for license detection
2. Integration tests for ingestion pipeline
3. End-to-end tests with sample repositories
4. Policy re-evaluation tests

**Documentation:**
1. Update README with new features
2. Create user guide for provenance queries
3. Document integration with #5979
4. Add ADR to `planning/designs/`

---

## Blockers and Dependencies

### Dependencies

| Dependency | Status | Notes |
|------------|--------|-------|
| Issue #6024 (parent) | ✅ OPEN | Licensing gate decision - parent issue |
| Issue #5979 (licensing tooling) | ❌ UNKNOWN | Must integrate with licensing family's tooling |
| Atlas ingestion system | ❌ NOT FOUND | Need to implement ingestion pipeline |
| Zone assignment policy | ❌ UNKNOWN | Need to find zone policy documentation |

### Questions Requiring Resolution

1. **Where does external repo ingestion currently happen?**
   - Need to identify ingestion entry point
   - May need to create new ingestion system

2. **What database stores this data?**
   - Atlas uses SQLite? Postgres?
   - Need to confirm storage backend

3. **What is the "zone assignment" policy?**
   - Need to find zone policy documentation
   - May need to define zone assignment logic

4. **What format should provenance records take?**
   - Need to finalize JSON schema
   - Need to define database schema

5. **How to integrate with #5979?**
   - Need to understand licensing tooling API
   - Need to coordinate integration

---

## Risk Assessment

| Risk | Level | Mitigation |
|------|-------|------------|
| License detection unreliable | Medium | Use multiple detection methods, fallback to "unknown", confidence scoring |
| Policy changes break re-evaluation | Low | Design flexible query interface, use parameterized queries |
| Performance impact on ingestion | Medium | Implement caching, async processing, batch ingestion |
| Missing parent issue context | Low | Coordinate with #6024 owner, attend design meetings |
| Integration with #5979 delayed | Medium | Create mock implementation, replace later |
| Database schema changes required | Medium | Design extensible schema, use migrations |

---

## Implementation Timeline

### Week 1-2: Provenance Schema & License Detection
- [ ] Design provenance schema (2 days)
- [ ] Implement license detection module (5 days)
- [ ] Create D.6 flag calculation logic (1 day)
- [ ] Design database schema (1 day)

### Week 3-4: Ingestion Pipeline
- [ ] Implement repository cloning (3 days)
- [ ] Create ingestion pipeline (5 days)
- [ ] Add provenance capture hooks (3 days)
- [ ] Create database migrations (1 day)

### Week 5-6: Policy Re-evaluation & Testing
- [ ] Implement re-evaluation queries (2 days)
- [ ] Create stored procedures (1 day)
- [ ] Write unit tests (3 days)
- [ ] Write integration tests (3 days)

### Week 7: Documentation & Deployment
- [ ] Create user documentation (2 days)
- [ ] Update README files (1 day)
- [ ] Create ADR document (1 day)
- [ ] Deploy to staging (1 day)

**Total Estimated Time:** 6-7 weeks (30-35 days)

---

## Technical Details

### License Detection Methods

1. **License File Detection**
   - Check for files: LICENSE, LICENCE, COPYING, UNLICENSE
   - Parse SPDX identifiers from file contents
   - Use regex patterns to identify license text

2. **Package Manifest Parsing**
   - Parse SPDX identifiers from:
     - package.json (npm)
     - Cargo.toml (Rust)
     - pyproject.toml (Python)
     - go.mod (Go)
     - Gemfile (Ruby)
     - etc.

3. **SPDX Tag Detection**
   - Search source files for SPDX tags:
     - `SPDX-License-Identifier: MIT`
     - `SPDX-FileCopyrightText: 2024 ...`
   - Use #5979 licensing tooling for advanced detection

4. **NOTICE File Parsing**
   - Check for NOTICE files that may contain license information
   - Parse attribution notices

### D.6 Applicability Flag Rules

**Flag = true when:**
- License file exists AND
- License is OSI-approved OR widely recognized SPDX identifier

**Flag = false when:**
- No license detected OR
- License is proprietary/non-standard OR
- License is unclear/ambiguous

**Rationale should include:**
- Which files were checked
- What license was detected (or "none")
- Confidence score
- Any notes about detection

### Zone Assignment Policy

Based on issue context, zone assignment is a "sibling policy decision". This likely means:
- Zones are predefined policy categories
- Zone assignment happens during ingestion
- Zone determines how repo is treated by policy layer

Possible zones:
- `public` - Public domain or permissive license
- `restricted` - Copyleft or conditional license
- `prohibited` - Non-permissive or proprietary license
- `review_required` - Unknown or ambiguous license
- `quarantine` - License requires legal review

---

## Integration Points

### 1. Atlas System Integration

Atlas appears to be a search/indexing system. Integration points:
- Atlas ingestion pipeline
- Atlas database storage
- Atlas query interface

### 2. Licensing Tooling (#5979)

Need to integrate with licensing family's tooling:
- Use their license detection API
- Use their SPDX database
- Coordinate on data formats

### 3. Policy Layer

Provenance records enable:
- Policy re-evaluation without re-crawling
- Filtering repos by license
- Filtering repos by D.6 flag
- Zone-based policy enforcement

---

## Testing Strategy

### Unit Tests

1. **License Detection Tests**
   - Test SPDX identifier parsing
   - Test license file detection
   - Test confidence scoring
   - Test fallback behavior

2. **D.6 Flag Tests**
   - Test flag calculation logic
   - Test edge cases (no license, unknown license)
   - Test confidence thresholds

3. **Provenance Schema Tests**
   - Test JSON schema validation
   - Test database schema
   - Test migration scripts

### Integration Tests

1. **Repository Ingestion Tests**
   - Test with sample repositories
   - Test with different license types
   - Test with no license
   - Test with ambiguous license

2. **Database Integration Tests**
   - Test CRUD operations
   - Test query performance
   - Test indexing

3. **Policy Re-evaluation Tests**
   - Test query results
   - Test stored procedures
   - Test edge cases

### End-to-End Tests

1. **Full Pipeline Test**
   - Clone external repo
   - Detect license
   - Calculate D.6 flag
   - Record provenance
   - Query provenance
   - Re-evaluate policy

2. **Performance Tests**
   - Test with 100+ repos
   - Test with large repos
   - Test caching behavior

---

## Success Criteria

### MVP (Minimum Viable Product)

- [ ] License detection works for common SPDX identifiers
- [ ] D.6 flag calculated correctly
- [ ] Provenance records stored in database
- [ ] Basic queries work
- [ ] Documentation created

### Full Implementation

- [ ] All SPDX identifiers supported
- [ ] Multiple detection methods (files, manifests, SPDX tags)
- [ ] Integration with #5979
- [ ] Zone assignment policy implemented
- [ ] Policy re-evaluation queries work
- [ ] Comprehensive tests
- [ ] User documentation
- [ ] Performance optimized

---

## Resources and References

### Issues
- **Issue #6053:** https://github.com/developing-today/code/issues/6053
- **Parent Issue #6024:** https://github.com/developing-today/code/issues/6024
- **Provenance Substrate #3125:** https://github.com/developing-today/code/issues/3125
- **Licensing Tooling #5979:** https://github.com/developing-today/code/issues/5979

### Research
- **Prior Art Doc:** `~/workspace/scratch/atlas-ingestion-brief-2026-11-04/prior-art-research.md`
- **SPDX License List:** https://spdx.org/licenses/
- **OSI License Approval:** https://opensource.org/licenses

### Code References
- **Provenance Matrix:** `/home/user/git/code/pm-stack/aggregator/scripts/check-provenance-matrix.ts`
- **Module Provenance:** `/home/user/git/code/doc/2026-10-16T09-00-00Z_feature_world_module_provenance/`
- **Ingestion Patterns:** `/home/user/git/code/src/mcpx/internal/logstore/ingest.go`

---

## Next Steps

### Immediate (This Task - Complete ✅)
- [x] Issue verified as OPEN
- [x] No blocking PRs found
- [x] Worktree prepared
- [x] Analysis document created
- [x] Labels updated

### Short-term (Next 2 weeks)
1. **Design provenance schema** - Create detailed schema documentation
2. **Implement license detection** - Create detection module
3. **Find ingestion entry point** - Identify where to hook into ingestion pipeline
4. **Design database schema** - Create migration scripts
5. **Create ADR** - Document design decision

### Medium-term (Next 4-6 weeks)
1. **Implement ingestion pipeline** - Create repository ingestion system
2. **Add provenance capture** - Hook into ingestion to record data
3. **Implement re-evaluation queries** - Create policy layer queries
4. **Write tests** - Unit, integration, and end-to-end tests
5. **Document** - User guides and API documentation

### Long-term (6+ weeks)
1. **Deploy to staging** - Test with real external repos
2. **Monitor accuracy** - Track license detection performance
3. **Refine based on usage** - Adjust based on actual patterns
4. **Integrate with #5979** - Connect with licensing tooling
5. **Optimize performance** - Improve ingestion speed

---

## Conclusion

**Issue #6053 requires a NEW implementation.** No existing code satisfies the requirements. The design needs to be created from scratch, including:

1. ✅ Provenance field schema (Phase 1)
2. ❌ License detection integration (Phase 2 - needs #5979)
3. ❌ Ingestion pipeline modifications (Phase 3 - need to find ingestion code)
4. ❌ Policy re-evaluation query (Phase 4)

**Recommendation:** Proceed with design phase, create implementation plan issue, and begin Phase 1 (provenance schema design) immediately.

---

**Analysis Date:** 2026-11-05  
**Analyst:** Subagent analysis complete  
**Task Status:** Analysis Complete - Ready for Implementation Planning
