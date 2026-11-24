#!/usr/bin/env python3
"""AUDIT-7 Lane 1 supplement: refetch full PR records (body/comments/files/state)
for every PR that was in the lane census window, all states, via fleet gh GraphQL.
Original in-session fetch scripts (fetch_bodies.py / fetch2.sh) were lost with /tmp
in a service restart on 2026-11-24; this is the recreated equivalent used for the
supplement refetch. Bodies are NOT stored in this script's output for merged PRs
beyond what the API returns; the file is the raw API response, unedited."""
import json, subprocess, os, sys
env=dict(os.environ); env['PATH']=os.path.expanduser('~/.local/share/hatch-recovery/bin')+':'+env['PATH']
Q=os.path.join(os.path.dirname(os.path.abspath(__file__)),'q3-full.graphql')
def gql(owner,name,cursor=None):
    args=['fleet','gh','api','graphql','-F',f'query=@{Q}','-f',f'owner={owner}','-f',f'name={name}']
    if cursor: args+=['-f',f'cursor={cursor}']
    for attempt in range(3):
        r=subprocess.run(args,capture_output=True,text=True,env=env)
        if r.returncode==0 and r.stdout.strip():
            return json.loads(r.stdout)['data']['repository']['pullRequests']
    raise SystemExit(f'FAIL {owner}/{name}: {r.stderr[:200]}')
out={}
for repo,owner,name in [('code','developing-today','code'),('opencode','developing-today-forks','opencode'),('openchamber','developing-today-forks','openchamber'),('fleet','dezren39','fleet')]:
    cur=None; alln=[]
    while True:
        d=gql(owner,name,cur); alln+=d['nodes']
        if not d['pageInfo']['hasNextPage']: break
        cur=d['pageInfo']['endCursor']
        if len(alln)>600: break  # supplement scope: recent window covering the census PRs
    out[repo]=alln; print(repo, len(alln), flush=True)
json.dump(out, open(os.path.join(os.path.dirname(os.path.abspath(__file__)),'pr-full-records-refetch.json'),'w'), indent=1)
print('saved')
