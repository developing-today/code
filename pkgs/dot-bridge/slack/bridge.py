#!/usr/bin/env python3
"""Private Slack bot bridge. Standard-library only; no automatic agent execution."""
import argparse, hashlib, hmac, json, os, re, sqlite3, time, uuid
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

MAX_BODY = 262144

def valid_signing_secret(secret):
    # Local configuration safety floor, not an assumed Slack hex encoding.
    # Slack documents the signing secret as an opaque UTF-8 string.
    return isinstance(secret,str) and len(secret)>=32 and secret.isprintable() and not any(c.isspace() for c in secret)

def verify(secret, timestamp, signature, body, now=None):
    if not valid_signing_secret(secret): return False
    try:
        if abs((time.time() if now is None else now) - int(timestamp)) > 300: return False
    except (TypeError, ValueError): return False
    expected = 'v0=' + hmac.new(secret.encode(), b'v0:' + timestamp.encode() + b':' + body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, signature or '')

class Bridge:
    def __init__(self, config, db):
        self.c = config
        for k in ('team_id','app_id','channel_id','bot_user_id','dot_user_id','allowed_sender_ids'):
            if not config.get(k): raise ValueError('missing config: '+k)
        for k in ('team_id','app_id','channel_id','bot_user_id','dot_user_id'):
            if not isinstance(config[k],str) or not re.fullmatch(r'[A-Z][A-Z0-9]+',config[k]):
                raise ValueError('invalid Slack identifier: '+k)
        if config['bot_user_id']==config['dot_user_id']: raise ValueError('Dot must be distinct from bridge bot')
        if not isinstance(config['allowed_sender_ids'],list) or any(not isinstance(x,str) or not re.fullmatch(r'U[A-Z0-9]+',x) for x in config['allowed_sender_ids']):
            raise ValueError('invalid sender allowlist')
        if config['dot_user_id'] not in config['allowed_sender_ids']: raise ValueError('Dot missing from allowlist')
        self.db = sqlite3.connect(db)
        self.db.row_factory = sqlite3.Row
        self.db.executescript('''
        CREATE TABLE IF NOT EXISTS outbound(id TEXT PRIMARY KEY, kind TEXT, thread_ts TEXT, text TEXT, status TEXT, ts TEXT);
        CREATE TABLE IF NOT EXISTS inbox(event_id TEXT PRIMARY KEY, message_ts TEXT UNIQUE, sender TEXT, thread_ts TEXT, request_id TEXT, text TEXT, processed INTEGER DEFAULT 0);
        ''')

    def api(self, method, data):
        token = os.environ['SLACK_BOT_TOKEN']
        if not token.startswith('xoxb-'): raise ValueError('only bot tokens are permitted')
        req = urllib.request.Request('https://slack.com/api/'+method, data=json.dumps(data).encode(),
            headers={'Authorization':'Bearer '+token,'Content-Type':'application/json; charset=utf-8'})
        with urllib.request.urlopen(req, timeout=15) as r: result=json.load(r)
        if not result.get('ok'): raise RuntimeError('Slack error: '+str(result.get('error','unknown')))
        return result

    def check(self):
        a=self.api('auth.test',{})
        if a.get('team_id')!=self.c['team_id'] or a.get('user_id')!=self.c['bot_user_id'] or not a.get('bot_id'):
            raise ValueError('token identity mismatch')
        ch=self.api('conversations.info',{'channel':self.c['channel_id']})['channel']
        if not ch.get('is_private') or not ch.get('is_member') or ch.get('is_archived') or ch.get('is_shared') or ch.get('is_ext_shared') or ch.get('is_org_shared'):
            raise ValueError('channel must be private, joined and not archived')
        return {'ok':True,'team_id':a['team_id'],'channel_id':ch['id'],'bot_user_id':a['user_id']}

    def receive(self, payload):
        if payload.get('team_id')!=self.c['team_id'] or payload.get('api_app_id')!=self.c['app_id']:
            raise ValueError('event tenant/app mismatch')
        if payload.get('type')!='event_callback': return 'ignored'
        e=payload.get('event',{}); sender=e.get('user')
        # Edits/deletes/broadcasts ignored; allow the configured Dot bot, never arbitrary bots.
        if e.get('type')!='message' or e.get('subtype') not in (None,'bot_message'): return 'ignored'
        if e.get('channel')!=self.c['channel_id'] or e.get('channel_type')!='group': return 'ignored'
        if sender==self.c['bot_user_id'] or sender not in self.c['allowed_sender_ids']: return 'ignored'
        ts=e.get('ts',''); text=e.get('text'); eid=payload.get('event_id')
        if not eid or not re.fullmatch(r'\d+\.\d+',ts) or not isinstance(text,str): raise ValueError('malformed message')
        thread=e.get('thread_ts') or ts
        request=self.db.execute("SELECT id FROM outbound WHERE kind='request' AND status='sent' AND ts=?",(thread,)).fetchone()
        # Persist early/unmatched replies too; correlation is assigned only from a sent receipt.
        # No inbound event triggers execution or a response.
        with self.db:
            cur=self.db.execute('INSERT OR IGNORE INTO inbox(event_id,message_ts,sender,thread_ts,request_id,text) VALUES(?,?,?,?,?,?)',
                (eid,ts,sender,thread,request['id'] if request else None,text))
        return 'stored' if cur.rowcount else 'duplicate'

    def send(self, text, request_id, reply_to=None):
        if not text.strip() or len(text)>12000: raise ValueError('text must be 1..12000 characters')
        uuid.UUID(request_id)
        thread=None; kind='request'
        if reply_to:
            row=self.db.execute('SELECT * FROM inbox WHERE event_id=?',(reply_to,)).fetchone()
            if not row: raise ValueError('reply must reference a stored authenticated event')
            thread=row['thread_ts']; kind='reply'
        safe_text=text.replace('&','&amp;').replace('<','&lt;').replace('>','&gt;')
        rendered=(f'<@{self.c["dot_user_id"]}> [request {request_id}]\n'+safe_text) if kind=='request' else safe_text
        old=self.db.execute('SELECT * FROM outbound WHERE id=?',(request_id,)).fetchone()
        if old:
            if old['text']!=rendered or old['thread_ts']!=thread: raise ValueError('idempotency key payload mismatch')
            if old['status']=='sent': return dict(old)
            raise ValueError('delivery pending/uncertain; reconcile Slack history, do not blindly resend')
        self.check()  # Read-only checks before every write, including private-channel membership.
        with self.db:
            self.db.execute('INSERT INTO outbound VALUES(?,?,?,?,?,NULL)',(request_id,kind,thread,rendered,'pending'))
        data={'channel':self.c['channel_id'],'text':rendered,'client_msg_id':request_id,'unfurl_links':False,'unfurl_media':False}
        if thread: data['thread_ts']=thread
        try:
            r=self.api('chat.postMessage',data)
            if r.get('channel')!=self.c['channel_id'] or not re.fullmatch(r'\d+\.\d+',r.get('ts','')): raise ValueError('unexpected send receipt')
        except Exception:
            with self.db: self.db.execute("UPDATE outbound SET status='uncertain' WHERE id=?",(request_id,))
            raise
        with self.db:
            self.db.execute("UPDATE outbound SET status='sent',ts=? WHERE id=?",(r['ts'],request_id))
            if kind=='request': self.db.execute('UPDATE inbox SET request_id=? WHERE thread_ts=? AND request_id IS NULL',(request_id,r['ts']))
        return {'id':request_id,'status':'sent','ts':r['ts']}

    def inbox(self):
        return [dict(r) for r in self.db.execute('SELECT * FROM inbox WHERE processed=0 ORDER BY message_ts')]
    def get_reply(self,request_id):
        return [dict(r) for r in self.db.execute('SELECT * FROM inbox WHERE request_id=? AND sender=? ORDER BY message_ts',(request_id,self.c['dot_user_id']))]
    def ack(self,eid):
        with self.db:
            c=self.db.execute('UPDATE inbox SET processed=1 WHERE event_id=?',(eid,))
            if not c.rowcount: raise ValueError('unknown event')

def handler(bridge,secret):
    if not valid_signing_secret(secret): raise ValueError('invalid Slack signing secret: require at least 32 printable non-whitespace characters')
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*args): pass  # No message text or credentials in HTTP logs.
        def do_POST(self):
            if self.path!='/slack/events': self.send_error(404); return
            try:
                if self.headers.get('Transfer-Encoding'): raise ValueError('chunked requests unsupported')
                n=int(self.headers.get('Content-Length','0'))
                if not 0<n<=MAX_BODY: raise ValueError('body size')
                self.connection.settimeout(5)
                body=self.rfile.read(n)
                if len(body)!=n: raise ValueError('short body')
                if not verify(secret,self.headers.get('X-Slack-Request-Timestamp'),self.headers.get('X-Slack-Signature'),body):
                    self.send_error(401); return
                p=json.loads(body)
                if not isinstance(p,dict): raise ValueError('payload shape')
                if p.get('type')=='url_verification': result={'challenge':p['challenge']}
                else: result={'result':bridge.receive(p)}
                out=json.dumps(result).encode(); self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(out))); self.end_headers(); self.wfile.write(out)
            except (ValueError,KeyError,TypeError): self.send_error(400)
            except Exception: self.send_error(503) # Slack retries; no ACK on failed durable store.
    return Handler

def main():
    os.umask(0o077)
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--config',required=True); p.add_argument('--db',default='bridge.sqlite3')
    sub=p.add_subparsers(dest='cmd',required=True)
    sub.add_parser('check'); sub.add_parser('inbox'); sub.add_parser('outbox')
    a=sub.add_parser('get-reply'); a.add_argument('request_id')
    a=sub.add_parser('ack'); a.add_argument('event_id')
    for name in ('request','reply'):
        a=sub.add_parser(name); a.add_argument('--id',required=True,help='stable UUID; reuse only for exactly same operation'); a.add_argument('--text-file',required=True)
        if name=='reply':a.add_argument('--event-id',required=True)
    a=sub.add_parser('serve'); a.add_argument('--port',type=int,default=8788)
    args=p.parse_args()
    secret=os.environ.get('SLACK_SIGNING_SECRET') if args.cmd=='serve' else None
    if args.cmd=='serve' and not valid_signing_secret(secret):
        raise ValueError('invalid Slack signing secret: require at least 32 printable non-whitespace characters')
    b=Bridge(json.load(open(args.config)),args.db)
    if args.cmd=='check': result=b.check()
    elif args.cmd=='inbox':result=b.inbox()
    elif args.cmd=='outbox':result=[dict(r) for r in b.db.execute('SELECT * FROM outbound')]
    elif args.cmd=='get-reply':result=b.get_reply(args.request_id)
    elif args.cmd=='ack':b.ack(args.event_id); result={'ok':True}
    elif args.cmd in ('request','reply'):result=b.send(open(args.text_file).read(),args.id,getattr(args,'event_id',None))
    else:
        b.check(); HTTPServer(('127.0.0.1',args.port),handler(b,secret)).serve_forever(); return
    print(json.dumps(result))
if __name__=='__main__': main()
