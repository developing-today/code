import hashlib,hmac,json,tempfile,unittest,uuid
from pathlib import Path
from bridge import Bridge,verify,handler,main
from unittest.mock import patch

C={'team_id':'T1','app_id':'A1','channel_id':'G1','bot_user_id':'UBRIDGE','dot_user_id':'UDOT','allowed_sender_ids':['UDOT','UDREW']}
def event(eid='Ev1',sender='UDREW',thread=None):
    e={'type':'message','channel':'G1','channel_type':'group','user':sender,'ts':'123.002','text':'hello'}
    if thread:e['thread_ts']=thread
    return {'type':'event_callback','team_id':'T1','api_app_id':'A1','event_id':eid,'event':e}
class TestBridge(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.b=Bridge(C,str(Path(self.tmp.name)/'db'))
        self.calls=[]
        def api(method,data):
            self.calls.append((method,data))
            if method=='auth.test':return {'team_id':'T1','user_id':'UBRIDGE','bot_id':'B1'}
            if method=='conversations.info':return {'channel':{'id':'G1','is_private':True,'is_member':True}}
            return {'channel':'G1','ts':'123.001'}
        self.b.api=api
    def tearDown(self):self.b.db.close();self.tmp.cleanup()
    def test_signature(self):
        body=b'{"x":1}';sig='v0='+hmac.new(b's'*32,b'v0:1000:'+body,hashlib.sha256).hexdigest()
        self.assertTrue(verify('s'*32,'1000',sig,body,1000));self.assertFalse(verify('s'*32,'1000',sig,body,1301));self.assertFalse(verify('s'*32,'1000',sig,b'other',1000))
    def test_invalid_signing_secret_fails_closed(self):
        body=b'{"x":1}'
        for secret in ('', ' '*32, 'short', 'x'*31, 'x'*32+'\n', 'x'*16+' '+ 'x'*16):
            with self.subTest(secret_length=len(secret)):
                sig='v0='+hmac.new(secret.encode(),b'v0:1000:'+body,hashlib.sha256).hexdigest()
                self.assertFalse(verify(secret,'1000',sig,body,1000))
                with self.assertRaisesRegex(ValueError,'signing secret'):
                    handler(self.b,secret)
                with patch('bridge.os.umask'), patch('sys.argv',['bridge.py','--config','must-not-open','serve']), patch.dict('os.environ',{'SLACK_SIGNING_SECRET':secret}):
                    with self.assertRaisesRegex(ValueError,'signing secret'):
                        main()
        self.assertFalse(verify(None,'1000','',body,1000))

    def test_identity_and_channel(self):
        p=event();p['team_id']='other'
        with self.assertRaises(ValueError):self.b.receive(p)
        p=event();p['event']['channel']='G2';self.assertEqual(self.b.receive(p),'ignored')
        p=event();p['event']['channel_type']='channel';self.assertEqual(self.b.receive(p),'ignored')
        self.assertEqual(self.b.receive(event(sender='OTHER')),'ignored')
    def test_ack_only_after_processing(self):
        self.assertEqual(self.b.receive(event()),'stored');self.assertEqual(self.b.receive(event()),'duplicate')
        self.assertEqual(len(self.b.inbox()),1);self.b.ack('Ev1');self.assertEqual(self.b.inbox(),[])
        self.assertEqual(self.b.receive(event()),'duplicate')
    def test_message_ts_dedup(self):
        self.b.receive(event());self.assertEqual(self.b.receive(event('Ev2')),'duplicate')
    def test_no_loops_no_auto_send(self):
        self.assertEqual(self.b.receive(event(sender='UBRIDGE')),'ignored')
        self.assertEqual(self.b.receive(event(sender='UDOT')),'stored')
        p=event();p['event']['subtype']='message_changed';self.assertEqual(self.b.receive(p),'ignored')
        self.assertEqual(self.calls,[])
    def test_request_reply_correlation(self):
        rid=str(uuid.uuid4());self.b.send('task',rid)
        p=event(sender='UDOT',thread='123.001');p['event']['bot_id']='BDOT'
        self.assertEqual(self.b.receive(p),'stored');self.assertEqual(self.b.inbox()[0]['request_id'],rid)
        self.b.send('answer',str(uuid.uuid4()),'Ev1')
        self.assertEqual(self.calls[-1][1]['thread_ts'],'123.001')
        self.assertFalse(self.calls[-1][1]['unfurl_links'])
    def test_send_idempotence(self):
        rid=str(uuid.uuid4());self.b.send('task',rid);n=len(self.calls);self.b.send('task',rid);self.assertEqual(len(self.calls),n)
        with self.assertRaises(ValueError):self.b.send('different',rid)
    def test_uncertain_never_retries(self):
        self.b.check=lambda:None
        def fail(*a):raise TimeoutError('network')
        self.b.api=fail;rid=str(uuid.uuid4())
        with self.assertRaises(TimeoutError):self.b.send('task',rid)
        self.assertEqual(self.b.db.execute('SELECT status FROM outbound').fetchone()[0],'uncertain')
        with self.assertRaises(ValueError):self.b.send('task',rid)
    def test_reply_requires_authenticated_inbox(self):
        with self.assertRaises(ValueError):self.b.send('answer',str(uuid.uuid4()),'unknown')
    def test_refuses_public_channel(self):
        original=self.b.api
        self.b.api=lambda m,d: {'channel':{'id':'G1','is_private':False,'is_member':True}} if m=='conversations.info' else original(m,d)
        with self.assertRaises(ValueError):self.b.send('task',str(uuid.uuid4()))
        self.assertFalse(any(x[0]=='chat.postMessage' for x in self.calls))
    def test_shared_channel_refused(self):
        original=self.b.api
        self.b.api=lambda m,d: {'channel':{'id':'G1','is_private':True,'is_member':True,'is_ext_shared':True}} if m=='conversations.info' else original(m,d)
        with self.assertRaises(ValueError): self.b.send('task',str(uuid.uuid4()))
    def test_markup_escape(self):
        self.b.send('<!channel> <@USER> <https://bad.example|click>',str(uuid.uuid4()))
        text=self.calls[-1][1]['text']
        self.assertTrue(text.startswith('<@UDOT>'))
        self.assertNotIn('<!channel>',text); self.assertNotIn('<@USER>',text);self.assertNotIn('<https:',text)
    def test_early_reply_persisted_and_correlated(self):
        self.b.receive(event(sender='UDOT',thread='123.001'))
        self.assertIsNone(self.b.inbox()[0]['request_id'])
        rid=str(uuid.uuid4());self.b.send('task',rid)
        self.assertEqual(len(self.b.get_reply(rid)),1)
        self.assertEqual(self.b.get_reply(rid)[0]['sender'],'UDOT')
if __name__=='__main__':unittest.main()
