#!/usr/bin/env python3
"""Offline lossless COPY archive. Usage: pg_copy_archive.py INPUT OUTPUT REPORT.
Strict argparse before I/O. Refuses existing outputs. No network, SQL execution
from the dump, type coercion or destination deployment. All fields remain TEXT
(or SQL NULL) to preserve Postgres lexical precision. This is recovery staging,
not a production schema migration; constraints/functions remain in source dump.
"""
import argparse, hashlib, json, os, re, sqlite3
from pathlib import Path

def quote(s): return '"'+s.replace('"','""')+'"'
def decode(s):
    if s == r'\N': return None
    out=bytearray(); b=s.encode('utf8'); i=0
    escapes={98:8,102:12,110:10,114:13,116:9,118:11}
    while i<len(b):
        c=b[i];i+=1
        if c!=92: out.append(c);continue
        if i==len(b): raise ValueError('trailing COPY escape')
        c=b[i];i+=1
        if c in escapes: out.append(escapes[c])
        elif 48<=c<=55:
            n=bytes([c]);
            while i<len(b) and len(n)<3 and 48<=b[i]<=55: n+=bytes([b[i]]);i+=1
            out.append(int(n,8))
        elif c==120:
            n=b''
            while i<len(b) and len(n)<2 and b[i] in b'0123456789abcdefABCDEF': n+=bytes([b[i]]);i+=1
            if not n: raise ValueError('invalid hex escape')
            out.append(int(n,16))
        else: out.append(c)
    return out.decode('utf8')

def digest_row(h,row):
    h.update(json.dumps(row,ensure_ascii=False,separators=(',',':')).encode()); h.update(b'\n')

def convert(src,dst,report):
    if dst.exists() or report.exists(): raise ValueError('output already exists')
    fd=os.open(dst,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600);os.close(fd)
    db=sqlite3.connect(dst); manifest={'format':'postgres-copy-lossless-text-v1','tables':{},'constraints_applied':False}
    table=None; h=None; batch=[]; sql=None; source_hash=hashlib.sha256()
    try:
        with src.open('rb') as f:
            for raw in f:
                source_hash.update(raw);line=raw.decode('utf8').rstrip('\n')
                if table:
                    if line==r'\.':
                        if batch: db.executemany(sql,batch);batch=[]
                        manifest['tables'][table]['sha256']=h.hexdigest(); table=None
                    else:
                        row=[decode(x) for x in line.split('\t')]
                        if len(row)!=len(cols): raise ValueError('COPY width mismatch')
                        digest_row(h,row);batch.append(row);manifest['tables'][table]['rows']+=1
                        if len(batch)>=1000: db.executemany(sql,batch);batch=[]
                elif line.startswith('COPY '):
                    m=re.fullmatch(r'COPY ([\w]+\.[\w]+) \(([^\n]+)\) FROM stdin;',line)
                    if not m: raise ValueError('unsupported COPY header')
                    table=m[1]
                    if not re.fullmatch(r'[a-zA-Z_][a-zA-Z_0-9]*(?:, [a-zA-Z_][a-zA-Z_0-9]*)*',m[2]): raise ValueError('unsupported COPY identifiers')
                    cols=m[2].split(', ')
                    if table in manifest['tables']: raise ValueError('duplicate COPY')
                    db.execute('CREATE TABLE '+quote(table)+' ('+','.join(quote(c)+' TEXT' for c in cols)+')')
                    sql='INSERT INTO '+quote(table)+' VALUES ('+','.join('?' for c in cols)+')'
                    manifest['tables'][table]={'columns':cols,'rows':0};h=hashlib.sha256()
        if table: raise ValueError('unterminated COPY')
        # Preserve DDL-only empty/partition-parent tables without executing DDL.
        ddl_name=None; ddl_cols=[]
        second_hash=hashlib.sha256()
        with src.open(newline='') as f:
            for line in f:
                second_hash.update(line.encode('utf8'))
                match=re.fullmatch(r'CREATE TABLE ([\w]+\.[\w]+) \(\n',line)
                if line.startswith('CREATE TABLE ') and not match: raise ValueError('unsupported CREATE TABLE header')
                if match: ddl_name=match[1];ddl_cols=[]
                elif ddl_name:
                    if line.startswith(')'):
                        if ddl_name not in manifest['tables']:
                            if not ddl_cols: raise ValueError('DDL-only table has no parsed columns')
                            db.execute('CREATE TABLE '+quote(ddl_name)+' ('+','.join(quote(c)+' TEXT' for c in ddl_cols)+')')
                            manifest['tables'][ddl_name]={'columns':ddl_cols,'rows':0,'sha256':hashlib.sha256().hexdigest(),'ddl_only':True}
                        ddl_name=None
                    elif line.startswith('    ') and not line.lstrip().startswith(('CONSTRAINT ', 'PRIMARY ', 'FOREIGN ', 'CHECK ', 'UNIQUE ')):
                        col=re.match(r'    ("(?:[^"]|"")+"|[a-zA-Z_][a-zA-Z_0-9]*) ',line)
                        if col and col[1].startswith(chr(34)): raise ValueError('unsupported quoted DDL column')
                        if col: ddl_cols.append(col[1])
        if ddl_name is not None: raise ValueError('unterminated CREATE TABLE')
        if second_hash.hexdigest()!=source_hash.hexdigest(): raise ValueError('source changed between passes')
        db.commit()
        for name,item in manifest['tables'].items():
            h=hashlib.sha256();count=0
            for row in db.execute('SELECT * FROM '+quote(name)+' ORDER BY rowid'): digest_row(h,list(row));count+=1
            if count!=item['rows'] or h.hexdigest()!=item['sha256']: raise ValueError('readback mismatch')
        assert db.execute('PRAGMA integrity_check').fetchone()==('ok',)
        manifest['source_sha256']=source_hash.hexdigest();manifest['total_rows']=sum(x['rows'] for x in manifest['tables'].values());manifest['readback_verified']=True
        with report.open('x') as f: json.dump(manifest,f,indent=2)
        os.chmod(report,0o600)
        return manifest
    finally: db.close()

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('input',type=Path);p.add_argument('output',type=Path);p.add_argument('report',type=Path);a=p.parse_args()
    m=convert(a.input,a.output,a.report);print(json.dumps({'tables':len(m['tables']),'rows':m['total_rows'],'readback_verified':m['readback_verified']}))
if __name__=='__main__':main()
