# Parent source identity audit

2026-11-11 UTC. GitHub operations in this audit used `gh api` only:
`repos/<repo>/commits/HEAD`, then `repos/<repo>/readme?ref=<sha>`.
Decoded README hashes identify source text; no product code was executed or
security behavior independently certified.

| Repository             | Pinned README commit                       | Decoded README SHA-256                                             |
| ---------------------- | ------------------------------------------ | ------------------------------------------------------------------ |
| mem0ai/mem0            | `f1791daa86d469249c9b002f62644d58a1eb96a0` | `f72db3426438dbdf6879a2b50583fbee8e7e2dde5d01308c6295bcd09fd3db86` |
| The-Vibe-Company/quivr | `bb1b19ecf4094dbd1e309e521ce8b5fed4fb539f` | `3096abed88a1b2bdbbec3d1696807906257002411235c010b9a25484323437a4` |
| khoj-ai/khoj           | `ae229ca894c0b80ad84664afcfdde523b5e87057` | `f43b29f1ad9d5792d4a2b1d46454c26249212c5f93e59923adc718463a3ac1ba` |
| screenpipe/screenpipe  | `79c05c3fcef76e53d5c55be9d9dd7eb06f5ad170` | `4a61e3072cab5dad3ab41be2ff08d589f902b3e3c194fa1aaeb414265dc1a61a` |
| open-webui/open-webui  | `b0e6098e940942281d9c83f3529ea117287a3a22` | `b13f7a8daa9b3c98297c5d0bbfc03f6371cf7343c84b204dde99690657591689` |

Separately, normal web requests retrieved the primary Mem search API and Quivr
2.0 known-limits pages on this date. Mutable hosted documentation remains dated
source evidence, not an immutable API or performance promise for Atlas.
