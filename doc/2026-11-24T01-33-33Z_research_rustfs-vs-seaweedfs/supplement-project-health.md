# Supplement: project health — raw GitHub API output

Read 2026-11-23/24 via `gh api`. Repository stats and contributor lists
returned 2026-11-23; release lists returned 2026-11-24 (same command,
completed after the main research pass).

## Repository stats

seaweedfs/seaweedfs:

```json
{"created":"2014-07-14T16:41:37Z","forks":3129,"license":"Apache-2.0","open_issues":715,"stars":36219}
```

rustfs/rustfs:

```json
{"created":"2023-11-23T13:45:10Z","forks":1686,"license":"Apache-2.0","open_issues":73,"stars":36435}
```

## Contributors (top 10, commit counts)

SeaweedFS:

```
chrislusf 11159
dependabot[bot] 1687
kmlebedev 393
chrisluuber 104
plisandro 102
github-actions[bot] 96
franchb 85
hilimd 54
LazyDBA247-Anyvision 54
alrs 52
```

RustFS:

```
overtrue 3348
houseme 1742
weisd 715
cxymds 467
marshawcoco 256
guojidan 249
reatang 194
loverustfs 178
GatewayJ 171
majinghe 166
```

## Releases (most recent first)

SeaweedFS — weekly-to-fortnightly cadence:

```
4.55 2026-11-23T03:01:13Z
4.54 2026-11-16T05:31:48Z
4.53 2026-11-08T18:10:39Z
4.52 2026-11-02T11:24:12Z
4.51 2026-10-25T19:39:12Z
```

RustFS — GA 2026-09-16, then roughly fortnightly patches:

```
1.0.6 2026-11-15T23:26:57Z
1.0.5 2026-11-10T17:55:02Z
1.0.4 2026-10-25T16:26:17Z
1.0.3 2026-10-17T15:49:09Z
1.0.2 2026-10-16T14:48:29Z
1.0.1 2026-10-03T04:58:30Z
1.0.0 2026-09-16T10:03:34Z
1.0.0-rc.6 2026-09-11T04:52:34Z
```

## Stewardship notes (web sources, checked 2026-11-23)

- SeaweedFS: Chris Lu (chrislusf, San Francisco) is the maintainer;
  Stackscape attributes ~68% of commits to the top contributor.
  Sponsor/Patreon-funded independent open source; the Enterprise edition is
  sold via seaweedfs.com. Stackscape counts 3 public adopters excluding the
  creator. Public production receipts: Kubeflow 1.11 and Epinio adopted
  SeaweedFS as their default S3 store replacing MinIO.
- RustFS: project claims 7 core engineers and 160+ contributors. Owner is
  RustFS, Inc. (founded 2024, holds the trademark). Geography/funding are
  opaque and contradictory in public sources: the about page gives an
  Atlanta, Georgia address; the USPTO trademark filing is by a Beijing
  company (Beijing The Ganges River Sand Technology Co., Ltd.);
  rustfs.com.cn exists; Stackscape lists "RustFS, Inc. — China". No
  published investors, funding, or enterprise support terms; no paid tier
  yet. Vendor deployment claims (2.7M at GA announcement; 3.7M+ later) are
  unverified vendor figures.
