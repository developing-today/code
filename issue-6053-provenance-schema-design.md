# Issue #6053: Provenance Schema Design Document

**Parent Issue:** #6053 - License capture at ingest  
**Design Phase:** Schema Design  
**Status:** Draft - Ready for Review

---

## 1. Overview

This document defines the provenance schema for recording license information and D.6 applicability flags for external repositories ingested into Atlas. The schema enables policy re-evaluation without requiring web re-crawling.

---

## 2. Requirements

### 2.1 Functional Requirements

From Issue #6053:
1. Record detected license (SPDX id where detectable)
2. Record D.6 inbound=outbound applicability flag
3. Record zone assignment
4. Record ingest date

### 2.2 Non-Functional Requirements

1. **Immutable Records:** Once recorded, provenance should not change (append-only)
2. **Query Performance:** Fast queries by license, D.6 flag, zone, and ingest date
3. **Extensibility:** Schema should support future fields without breaking changes
4. **Validation:** Data should be validated before storage
5. **Auditability:** Full history of changes should be available

---

## 3. Data Model

### 3.1 Core Provenance Record

```typescript
interface ExternalRepoProvenance {
  // Required fields
  id: string; // UUID v4
  repo_url: string; // Canonical URL (https://github.com/owner/repo)
  detected_license: LicenseInfo | null;
  d6_applicability: D6Flag;
  zone_assignment: string; // Policy zone identifier
  ingest_date: string; // ISO 8601 timestamp
  
  // Optional fields
  source_revision?: string; // Commit hash, tag, or branch
  detected_license_files?: string[]; // Paths to license files found
  confidence_score?: number; // 0-1 confidence in detection
  detection_method?: DetectionMethod; // How license was detected
  notes?: string; // Human-readable notes
  
  // Metadata
  created_at?: string; // When record was created
  updated_at?: string; // When record was last updated
  version?: string; // Schema version
}
```

### 3.2 LicenseInfo Structure

```typescript
interface LicenseInfo {
  spdx_id: string; // SPDX identifier (e.g., "MIT", "Apache-2.0")
  full_name?: string; // Human-readable name
  url?: string; // SPDX license URL
  osi_approved?: boolean; // OSI approved status
  fsf_free?: boolean; // FSF Free Software status
  detected_from: string; // File path or detection method
  version?: string; // License version if available
  confidence: number; // 0-1 confidence score
}
```

### 3.3 D6Flag Structure

```typescript
interface D6Flag {
  flag: boolean; // true if repo carries license notice → comments under same terms
  rationale: string; // Explanation of detection
  detected_files?: string[]; // Files that contributed to decision
  confidence: number; // 0-1 confidence in flag value
}
```

---

## 4. Database Schema

### 4.1 SQLite Schema

```sql
-- Main provenance table
CREATE TABLE external_repo_provenance (
  id TEXT PRIMARY KEY, -- UUID v4
  repo_url TEXT NOT NULL, -- Canonical URL
  detected_license_spdx TEXT, -- SPDX identifier
  detected_license_full_name TEXT, -- Human-readable name
  detected_license_url TEXT, -- SPDX license URL
  detected_license_osi_approved INTEGER, -- 0 or 1
  detected_license_fsf_free INTEGER, -- 0 or 1
  detected_license_confidence REAL, -- 0-1
  detected_license_from TEXT, -- Detection source
  detected_license_version TEXT, -- License version
  
  d6_flag BOOLEAN NOT NULL, -- D.6 applicability flag
  d6_rationale TEXT, -- Rationale for flag
  d6_detected_files TEXT, -- JSON array of detected files
  d6_confidence REAL, -- 0-1 confidence
  
  zone_assignment TEXT NOT NULL, -- Policy zone
  
  ingest_date TEXT NOT NULL, -- ISO 8601 timestamp
  source_revision TEXT, -- Commit hash/tag
  detected_license_files TEXT, -- JSON array of license file paths
  confidence_score REAL, -- Overall confidence 0-1
  detection_method TEXT, -- Detection method used
  
  notes TEXT, -- Human notes
  
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  schema_version TEXT NOT NULL DEFAULT '1.0'
);

-- Indexes for fast queries
CREATE INDEX idx_provenance_repo_url ON external_repo_provenance(repo_url);
CREATE INDEX idx_provenance_license ON external_repo_provenance(detected_license_spdx);
CREATE INDEX idx_provenance_d6_flag ON external_revenance_provenance(d6_flag);
CREATE INDEX idx_provenance_zone ON external_repo_provenance(zone_assignment);
CREATE INDEX idx_provenance_ingest_date ON external_repo_provenance(ingest_date);
CREATE INDEX idx_provenance_ingest_date_zone ON external_repo_provenance(ingest_date, zone_assignment);
CREATE INDEX idx_provenance_license_zone ON external_repo_provenance(detected_license_spdx, zone_assignment);

-- Unique constraint to prevent duplicates
CREATE UNIQUE INDEX idx_provenance_repo_url_unique ON external_repo_provenance(repo_url);
```

### 4.2 PostgreSQL Schema

```sql
-- Main provenance table
CREATE TABLE external_repo_provenance (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  repo_url TEXT NOT NULL,
  detected_license_spdx TEXT,
  detected_license_full_name TEXT,
  detected_license_url TEXT,
  detected_license_osi_approved BOOLEAN,
  detected_license_fsf_free BOOLEAN,
  detected_license_confidence REAL,
  detected_license_from TEXT,
  detected_license_version TEXT,
  
  d6_flag BOOLEAN NOT NULL,
  d6_rationale TEXT,
  d6_detected_files TEXT[], -- Array of file paths
  d6_confidence REAL,
  
  zone_assignment TEXT NOT NULL,
  
  ingest_date TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  source_revision TEXT,
  detected_license_files TEXT[], -- Array of license file paths
  confidence_score REAL,
  detection_method TEXT,
  
  notes TEXT,
  
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  schema_version TEXT NOT NULL DEFAULT '1.0'
);

-- Indexes
CREATE INDEX idx_provenance_repo_url ON external_repo_provenance(repo_url);
CREATE INDEX idx_provenance_license ON external_repo_provenance(detected_license_spdx);
CREATE INDEX idx_provenance_d6_flag ON external_repo_provenance(d6_flag);
CREATE INDEX idx_provenance_zone ON external_repo_provenance(zone_assignment);
CREATE INDEX idx_provenance_ingest_date ON external_repo_provenance(ingest_date);
CREATE INDEX idx_provenance_ingest_date_zone ON external_repo_provenance(ingest_date, zone_assignment);
CREATE INDEX idx_provenance_license_zone ON external_repo_provenance(detected_license_spdx, zone_assignment);

-- Unique constraint
CREATE UNIQUE INDEX idx_provenance_repo_url_unique ON external_repo_provenance(repo_url);
```

---

## 5. JSON Schema Validation

### 5.1 Schema Definition

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://developing.today/schemas/external-repo-provenance-v1.json",
  "title": "External Repository Provenance Record",
  "description": "Provenance information for external repositories ingested into Atlas",
  "type": "object",
  "required": [
    "id",
    "repo_url",
    "d6_applicability",
    "zone_assignment",
    "ingest_date"
  ],
  "properties": {
    "id": {
      "type": "string",
      "format": "uuid",
      "description": "Unique identifier for the provenance record"
    },
    "repo_url": {
      "type": "string",
      "format": "uri",
      "pattern": "^https?://.*",
      "description": "Canonical repository URL"
    },
    "detected_license": {
      "type": ["object", "null"],
      "description": "Detected license information",
      "properties": {
        "spdx_id": {
          "type": "string",
          "description": "SPDX license identifier"
        },
        "full_name": {
          "type": "string"
        },
        "url": {
          "type": "string",
          "format": "uri"
        },
        "osi_approved": {
          "type": "boolean"
        },
        "fsf_free": {
          "type": "boolean"
        },
        "detected_from": {
          "type": "string"
        },
        "version": {
          "type": "string"
        },
        "confidence": {
          "type": "number",
          "minimum": 0,
          "maximum": 1
        }
      },
      "required": ["spdx_id", "detected_from", "confidence"]
    },
    "d6_applicability": {
      "type": "object",
      "description": "D.6 inbound=outbound applicability flag",
      "required": ["flag", "rationale"],
      "properties": {
        "flag": {
          "type": "boolean",
          "description": "True if repo carries license notice → comments under same terms"
        },
        "rationale": {
          "type": "string",
          "description": "Explanation of detection"
        },
        "detected_files": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "confidence": {
          "type": "number",
          "minimum": 0,
          "maximum": 1
        }
      }
    },
    "zone_assignment": {
      "type": "string",
      "description": "Policy zone identifier"
    },
    "ingest_date": {
      "type": "string",
      "format": "date-time",
      "description": "When the repository was ingested"
    },
    "source_revision": {
      "type": "string",
      "description": "Commit hash, tag, or branch"
    },
    "detected_license_files": {
      "type": "array",
      "items": {
        "type": "string"
      }
    },
    "confidence_score": {
      "type": "number",
      "minimum": 0,
      "maximum": 1
    },
    "detection_method": {
      "type": "string",
      "enum": ["license_file", "package_manifest", "spdx_tags", "license_tooling", "mixed"]
    },
    "notes": {
      "type": "string"
    },
    "created_at": {
      "type": "string",
      "format": "date-time"
    },
    "updated_at": {
      "type": "string",
      "format": "date-time"
    },
    "schema_version": {
      "type": "string",
      "default": "1.0"
    }
  },
  "additionalProperties": false
}
```

---

## 6. Example Records

### 6.1 MIT License - Clear Detection

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "repo_url": "https://github.com/facebook/react",
  "detected_license": {
    "spdx_id": "MIT",
    "full_name": "MIT License",
    "url": "https://spdx.org/licenses/MIT.html",
    "osi_approved": true,
    "fsf_free": true,
    "detected_from": "LICENSE",
    "version": null,
    "confidence": 0.98
  },
  "d6_applicability": {
    "flag": true,
    "rationale": "LICENSE file detected with MIT SPDX identifier",
    "detected_files": ["LICENSE"],
    "confidence": 0.98
  },
  "zone_assignment": "public",
  "ingest_date": "2026-11-05T18:30:45Z",
  "source_revision": "v18.2.0",
  "detected_license_files": ["LICENSE"],
  "confidence_score": 0.98,
  "detection_method": "license_file",
  "created_at": "2026-11-05T18:30:45Z",
  "updated_at": "2026-11-05T18:30:45Z",
  "schema_version": "1.0"
}
```

### 6.2 Apache-2.0 License

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440001",
  "repo_url": "https://github.com/apache/kafka",
  "detected_license": {
    "spdx_id": "Apache-2.0",
    "full_name": "Apache License 2.0",
    "url": "https://spdx.org/licenses/Apache-2.0.html",
    "osi_approved": true,
    "fsf_free": true,
    "detected_from": "LICENSE",
    "version": "2.0",
    "confidence": 0.99
  },
  "d6_applicability": {
    "flag": true,
    "rationale": "LICENSE file detected with Apache-2.0 SPDX identifier",
    "detected_files": ["LICENSE"],
    "confidence": 0.99
  },
  "zone_assignment": "public",
  "ingest_date": "2026-11-05T18:31:12Z",
  "source_revision": "3.7.0",
  "detected_license_files": ["LICENSE"],
  "confidence_score": 0.99,
  "detection_method": "license_file",
  "notes": "Large enterprise project with well-maintained licensing",
  "schema_version": "1.0"
}
```

### 6.3 No License Detected

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440002",
  "repo_url": "https://github.com/unknown/private-repo",
  "detected_license": null,
  "d6_applicability": {
    "flag": false,
    "rationale": "No license file detected in repository",
    "confidence": 0.95
  },
  "zone_assignment": "review_required",
  "ingest_date": "2026-11-05T18:32:03Z",
  "source_revision": "main",
  "detected_license_files": [],
  "confidence_score": 0.0,
  "detection_method": "license_file",
  "notes": "Repository may be private or license unclear",
  "schema_version": "1.0"
}
```

### 6.4 Ambiguous License (Multiple Files)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440003",
  "repo_url": "https://github.com/mixed-license/example",
  "detected_license": {
    "spdx_id": "MIT",
    "full_name": "MIT License",
    "url": "https://spdx.org/licenses/MIT.html",
    "osi_approved": true,
    "fsf_free": true,
    "detected_from": "package.json",
    "version": null,
    "confidence": 0.85
  },
  "d6_applicability": {
    "flag": true,
    "rationale": "Multiple license indicators detected (package.json SPDX, LICENSE file)",
    "detected_files": ["package.json", "LICENSE"],
    "confidence": 0.85
  },
  "zone_assignment": "public",
  "ingest_date": "2026-11-05T18:33:22Z",
  "source_revision": "v1.0.0",
  "detected_license_files": ["package.json", "LICENSE"],
  "confidence_score": 0.85,
  "detection_method": "mixed",
  "notes": "Multiple license indicators found, MIT selected as primary",
  "schema_version": "1.0"
}
```

---

## 7. Detection Methods

### 7.1 License File Detection

**Priority Order:**
1. `LICENSE`
2. `LICENCE`
3. `COPYING`
4. `UNLICENSE`
5. `LICENSE.md`
6. `LICENSE.txt`
7. `COPYRIGHT`
8. `NOTICE`

**Detection Logic:**
```typescript
const LICENSE_FILES = [
  'LICENSE',
  'LICENCE',
  'COPYING',
  'UNLICENSE',
  'LICENSE.md',
  'LICENSE.txt',
  'COPYRIGHT',
  'NOTICE'
];

function detectLicenseFromFiles(repoPath: string): LicenseDetectionResult {
  for (const filename of LICENSE_FILES) {
    const filePath = path.join(repoPath, filename);
    if (fs.existsSync(filePath)) {
      const content = fs.readFileSync(filePath, 'utf8');
      const spdxMatch = extractSPDX(content);
      if (spdxMatch) {
        return {
          spdx_id: spdxMatch.id,
          confidence: spdxMatch.confidence,
          detected_from: filename,
          version: spdxMatch.version
        };
      }
    }
  }
  return null;
}
```

### 7.2 Package Manifest Parsing

**Supported Manifests:**
- `package.json` (npm/yarn)
- `Cargo.toml` (Rust/Cargo)
- `pyproject.toml` (Python/Poetry)
- `go.mod` (Go)
- `Gemfile` (Ruby/Bundler)
- `composer.json` (PHP/Composer)
- `pubspec.yaml` (Dart/Flutter)
- `mix.exs` (Elixir)

**Detection Logic:**
```typescript
function detectLicenseFromManifest(repoPath: string): LicenseDetectionResult {
  const manifestFiles = [
    'package.json',
    'Cargo.toml',
    'pyproject.toml',
    'go.mod',
    'Gemfile',
    'composer.json',
    'pubspec.yaml',
    'mix.exs'
  ];
  
  for (const manifestFile of manifestFiles) {
    const filePath = path.join(repoPath, manifestFile);
    if (fs.existsSync(filePath)) {
      const content = fs.readFileSync(filePath, 'utf8');
      const licenseMatch = extractLicenseFromManifest(content, manifestFile);
      if (licenseMatch) {
        return licenseMatch;
      }
    }
  }
  return null;
}
```

### 7.3 SPDX Tag Detection

**Search Patterns:**
- `SPDX-License-Identifier: <SPDX_ID>`
- `SPDX-FileCopyrightText: <YEAR> <HOLDER>`
- `SPDX-License-Identifier: <SPDX_ID>`

**Detection Logic:**
```typescript
function detectSPDXTags(repoPath: string): LicenseDetectionResult {
  const files = walkDirectory(repoPath);
  
  for (const file of files) {
    if (isSourceFile(file)) {
      const content = fs.readFileSync(file, 'utf8');
      const spdxMatches = extractSPDXFromContent(content);
      if (spdxMatches.length > 0) {
        // Select best match
        return spdxMatches[0];
      }
    }
  }
  return null;
}
```

### 7.4 License Tooling Integration (#5979)

```typescript
async function detectWithLicenseTooling(repoPath: string): Promise<LicenseDetectionResult> {
  // Call #5979 licensing tooling
  const result = await callLicenseDetectionTool(repoPath);
  
  return {
    spdx_id: result.spdx_id,
    confidence: result.confidence,
    detected_from: 'license_tooling',
    version: result.version,
    files_checked: result.files_checked
  };
}
```

---

## 8. D.6 Applicability Flag Logic

### 8.1 Flag Calculation

```typescript
function calculateD6Flag(licenseResult: LicenseDetectionResult | null): D6Flag {
  if (!licenseResult) {
    return {
      flag: false,
      rationale: 'No license detected',
      confidence: 1.0
    };
  }
  
  const isRecognized = isRecognizedLicense(licenseResult.spdx_id);
  const confidence = licenseResult.confidence || 0.8;
  
  return {
    flag: isRecognized,
    rationale: isRecognized
      ? `License ${licenseResult.spdx_id} is recognized`
      : `License ${licenseResult.spdx_id} is not recognized`,
    detected_files: licenseResult.files_checked || [],
    confidence: confidence
  };
}

function isRecognizedLicense(spdxId: string | null): boolean {
  if (!spdxId) return false;
  
  // List of recognized licenses (OSI-approved + widely used)
  const RECOGNIZED_LICENSES = new Set([
    'MIT', 'Apache-2.0', 'GPL-2.0', 'GPL-3.0', 'LGPL-2.1', 'LGPL-3.0',
    'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'MPL-2.0', 'Unlicense',
    'CC0-1.0', 'AGPL-3.0', 'EPL-2.0'
  ]);
  
  return RECOGNIZED_LICENSES.has(spdxId);
}
```

### 8.2 Confidence Scoring

**Confidence Levels:**
- **0.9-1.0:** Strong detection (license file with SPDX identifier)
- **0.7-0.89:** Medium detection (package manifest with SPDX)
- **0.5-0.69:** Weak detection (SPDX tags in source, ambiguous file)
- **0.0-0.49:** Low confidence (unknown license, no clear indicators)

---

## 9. Zone Assignment Policy

### 9.1 Zone Definitions

```typescript
const ZONE_POLICY = {
  PUBLIC: {
    id: 'public',
    description: 'Permissive or public domain licenses',
    criteria: (licenseInfo) => {
      if (!licenseInfo) return false;
      const publicLicenses = new Set([
        'MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause',
        'ISC', 'Unlicense', 'CC0-1.0', 'WTFPL'
      ]);
      return publicLicenses.has(licenseInfo.spdx_id);
    }
  },
  RESTRICTED: {
    id: 'restricted',
    description: 'Copyleft or conditional licenses',
    criteria: (licenseInfo) => {
      if (!licenseInfo) return false;
      const restrictedLicenses = new Set([
        'GPL-2.0', 'GPL-3.0', 'LGPL-2.1', 'LGPL-3.0',
        'AGPL-3.0', 'MPL-2.0', 'EPL-2.0'
      ]);
      return restrictedLicenses.has(licenseInfo.spdx_id);
    }
  },
  PROHIBITED: {
    id: 'prohibited',
    description: 'Non-permissive or proprietary licenses',
    criteria: (licenseInfo) => {
      if (!licenseInfo) return false;
      const prohibitedLicenses = new Set([
        'GPL-3.0-only', 'AGPL-3.0-only', 'Commercial'
      ]);
      return prohibitedLicenses.has(licenseInfo.spdx_id);
    }
  },
  REVIEW_REQUIRED: {
    id: 'review_required',
    description: 'Unknown, ambiguous, or requires legal review',
    criteria: (licenseInfo, d6Flag) => {
      return !licenseInfo || d6Flag.flag === false;
    }
  },
  QUARANTINE: {
    id: 'quarantine',
    description: 'Licenses requiring immediate legal review',
    criteria: (licenseInfo) => {
      if (!licenseInfo) return false;
      const quarantineLicenses = new Set([
        'Proprietary', 'Custom', 'Unknown'
      ]);
      return quarantineLicenses.has(licenseInfo.spdx_id);
    }
  }
};
```

### 9.2 Zone Assignment Logic

```typescript
function assignZone(licenseInfo: LicenseDetectionResult | null, d6Flag: D6Flag): string {
  // Check in priority order
  if (ZONE_POLICY.QUARANTINE.criteria(licenseInfo)) {
    return ZONE_POLICY.QUARANTINE.id;
  }
  
  if (ZONE_POLICY.PROHIBITED.criteria(licenseInfo)) {
    return ZONE_POLICY.PROHIBITED.id;
  }
  
  if (ZONE_POLICY.RESTRICTED.criteria(licenseInfo)) {
    return ZONE_POLICY.RESTRICTED.id;
  }
  
  if (ZONE_POLICY.PUBLIC.criteria(licenseInfo)) {
    return ZONE_POLICY.PUBLIC.id;
  }
  
  // Default to review_required
  return ZONE_POLICY.REVIEW_REQUIRED.id;
}
```

---

## 10. API Design

### 10.1 Record Provenance API

**Endpoint:** `POST /api/v1/provenance/external-repo`

**Request:**
```json
{
  "repo_url": "https://github.com/owner/repo",
  "source_revision": "v1.0.0"
}
```

**Response:**
```json
{
  "status": "success",
  "record": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "repo_url": "https://github.com/owner/repo",
    "detected_license": {...},
    "d6_applicability": {...},
    "zone_assignment": "public",
    "ingest_date": "2026-11-05T18:30:45Z"
  }
}
```

### 10.2 Query Provenance API

**Endpoint:** `GET /api/v1/provenance/external-repo`

**Query Parameters:**
- `license` - Filter by SPDX ID
- `d6_flag` - Filter by D.6 flag (true/false)
- `zone` - Filter by zone assignment
- `from_date` - Start date (ISO 8601)
- `to_date` - End date (ISO 8601)
- `limit` - Maximum results (default: 100)
- `offset` - Pagination offset (default: 0)

**Example Request:**
```
GET /api/v1/provenance/external-repo?license=MIT&zone=public&limit=50
```

**Example Response:**
```json
{
  "results": [
    {
      "id": "...",
      "repo_url": "https://github.com/facebook/react",
      "detected_license": {"spdx_id": "MIT", "confidence": 0.98},
      "d6_applicability": {"flag": true},
      "zone_assignment": "public",
      "ingest_date": "2026-11-05T18:30:45Z"
    }
  ],
  "total": 1,
  "limit": 50,
  "offset": 0
}
```

### 10.3 Re-evaluation API

**Endpoint:** `POST /api/v1/provenance/external-repo/re-evaluate`

**Request:**
```json
{
  "query": "SELECT * FROM external_repo_provenance WHERE detected_license_spdx IS NULL",
  "policy_changes": {
    "new_zones": {...},
    "license_updates": {...}
  }
}
```

**Response:**
```json
{
  "status": "success",
  "affected_rows": 5,
  "updated_records": [...]
}
```

---

## 11. Migration Strategy

### 11.1 Schema Versioning

- **Version 1.0:** Initial schema (current)
- Future versions will be backward compatible
- Use `schema_version` field to track

### 11.2 Migration Scripts

**SQLite Migration:**
```sql
-- Migration from v1.0 to v2.0
ALTER TABLE external_repo_provenance ADD COLUMN new_field TEXT;
```

**PostgreSQL Migration:**
```sql
ALTER TABLE external_repo_provenance ADD COLUMN new_field TEXT;
```

---

## 12. Testing Strategy

### 12.1 Unit Tests

**License Detection Tests:**
- Test SPDX identifier extraction
- Test license file detection
- Test package manifest parsing
- Test confidence scoring
- Test fallback behavior

**D.6 Flag Tests:**
- Test flag calculation for different license types
- Test edge cases (no license, unknown license)
- Test confidence thresholds

**Zone Assignment Tests:**
- Test zone assignment for all license types
- Test priority order
- Test default behavior

### 12.2 Integration Tests

**Database Tests:**
- Test CRUD operations
- Test query performance
- Test indexing
- Test validation

**API Tests:**
- Test record creation
- Test query endpoints
- Test error handling

### 12.3 End-to-End Tests

**Full Pipeline Test:**
1. Clone external repository
2. Detect license
3. Calculate D.6 flag
4. Assign zone
5. Record provenance
6. Query provenance
7. Re-evaluate policy

---

## 13. Performance Considerations

### 13.1 Indexing Strategy

- **Primary Index:** `repo_url` (unique)
- **Secondary Indexes:** `detected_license_spdx`, `d6_flag`, `zone_assignment`, `ingest_date`
- **Composite Indexes:** `ingest_date + zone`, `license + zone`

### 13.2 Query Optimization

```sql
-- Fast query by license
SELECT * FROM external_repo_provenance 
WHERE detected_license_spdx = 'MIT' 
ORDER BY ingest_date DESC 
LIMIT 100;

-- Fast query by zone and date range
SELECT * FROM external_repo_provenance 
WHERE zone_assignment = 'public' 
  AND ingest_date >= '2026-10-01'
ORDER BY ingest_date DESC;
```

### 13.3 Caching Strategy

- Cache frequent queries (e.g., count by license type)
- Cache license detection results for same repository
- Use Redis or in-memory cache for hot data

---

## 14. Security Considerations

### 14.1 Data Validation

- Validate `repo_url` format
- Validate SPDX identifiers against known list
- Validate zone assignments
- Sanitize all inputs

### 14.2 Access Control

- Only authorized services can write provenance
- Read access for policy layer and auditing
- Audit logging for all changes

---

## 15. Monitoring and Metrics

### 15.1 Key Metrics

- **Total records:** Count of provenance records
- **License distribution:** Count by SPDX ID
- **Zone distribution:** Count by zone
- **D.6 flag distribution:** Count by flag value
- **Detection confidence:** Average confidence score
- **Ingestion rate:** Records per hour/day

### 15.2 Alerts

- Low confidence detections (< 0.5)
- High error rates in ingestion
- Unusual license types
- Zone assignment changes

---

## 16. Future Extensions

### 16.1 Planned Features

1. **License compatibility checking** - Check if licenses are compatible
2. **Dependency license tracking** - Track licenses of dependencies
3. **Automated license review** - Flag problematic licenses
4. **License change detection** - Detect when license changes
5. **Multi-license support** - Handle repositories with multiple licenses

### 16.2 Schema Extensions

```typescript
// Future fields
{
  "dependency_licenses": [...], // Array of dependency licenses
  "license_compatibility": {...}, // Compatibility information
  "legal_review_status": "pending|approved|rejected", // Legal review
  "review_notes": "...", // Legal review notes
  "expiration_date": "..." // License expiration if applicable
}
```

---

## 17. References

### 17.1 SPDX Resources
- SPDX License List: https://spdx.org/licenses/
- SPDX Specification: https://spdx.dev/specifications/
- SPDX License XML: https://github.com/spdx/license-list-data

### 17.2 License Detection Tools
- Licensee: https://github.com/licensee/licensee
- License Picker: https://github.com/license-picker/license-picker
- FOSSA License Scanner: https://github.com/fossas/fossa-cli
- #5979 Licensing Tooling

### 17.3 Related Standards
- REUSE Specification: https://reuse.software/spec/
- ClearlyDefined: https://clearlydefined.io/
- Open Source Licensing Guide: https://opensource.org/licenses

---

## 18. Appendix

### 18.1 Common SPDX Identifiers

```
MIT
Apache-2.0
GPL-2.0-only
GPL-2.0-or-later
GPL-3.0-only
GPL-3.0-or-later
LGPL-2.1-only
LGPL-2.1-or-later
LGPL-3.0-only
LGPL-3.0-or-later
BSD-2-Clause
BSD-3-Clause
ISC
MPL-2.0
Unlicense
CC0-1.0
AGPL-3.0-only
AGPL-3.0-or-later
EPL-2.0
```

### 18.2 Zone Assignment Quick Reference

| License | Zone | D.6 Flag |
|---------|------|----------|
| MIT | public | true |
| Apache-2.0 | public | true |
| BSD-2-Clause | public | true |
| GPL-2.0 | restricted | true |
| GPL-3.0 | restricted | true |
| AGPL-3.0 | restricted | true |
| No license | review_required | false |
| Unknown license | review_required | false |

---

**Document Version:** 1.0  
**Last Updated:** 2026-11-05  
**Status:** Draft - Ready for Review  
**Next Steps:** Review with team, finalize schema, begin implementation