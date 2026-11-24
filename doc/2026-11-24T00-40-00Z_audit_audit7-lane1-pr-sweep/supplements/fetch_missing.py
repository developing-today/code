#!/usr/bin/env python3
"""Targeted refetch for census PRs older than the DESC windows in fetch_full.py /
fetch_merged.py (per-number GraphQL pullRequest query), unioned into the same JSON."""
import json, subprocess, os
env=dict(os.environ); env['PATH']=os.path.expanduser('~/.local/share/hatch-recovery/bin')+':'+env['PATH']
HERE=os.path.dirname(os.path.abspath(__file__))
Q='query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){number title state merged isDraft mergeable mergeStateStatus createdAt updatedAt closedAt mergedAt changedFiles additions deletions author{login} baseRefName headRefName comments(first:20){totalCount nodes{author{login} bodyText createdAt}} files(first:100){nodes{path}}}}}'
open(os.path.join(HERE,'q4-one.graphql'),'w').write(Q)
def gql(owner,name,number):
    args=['fleet','gh','api','graphql','-F',f'query=@{os.path.join(HERE,"q4-one.graphql")}','-f',f'owner={owner}','-f',f'name={name}','-F',f'number={number}']
    for attempt in range(4):
        r=subprocess.run(args,capture_output=True,text=True,env=env)
        if r.returncode==0 and r.stdout.strip():
            return json.loads(r.stdout)['data']['repository']['pullRequest']
    raise SystemExit(f'FAIL {name}#{number}: {r.stderr[:200]}')
path=os.path.join(HERE,'pr-full-records-refetch.json'); out=json.load(open(path))
missing={'code':[13041,13032,13030,13012,12989,12948,12912,12887,12861,12795,12793,12771,12750,12745,12663,12662,12666,12682,12827]}
owners={'code':('developing-today','code')}
for repo,nums in missing.items():
    owner,name=owners[repo]; by={p['number']:p for p in out[repo]}
    for n in nums:
        p=gql(owner,name,n); by[n]=p; print(repo,n,p['state'],'merged=',p['merged'],flush=True)
    out[repo]=sorted(by.values(), key=lambda p:p['number'], reverse=True)
json.dump(out, open(path,'w'), indent=1); print('saved')
