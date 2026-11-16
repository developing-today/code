const enc=new TextEncoder();
export const hex=b=>[...new Uint8Array(b)].map(x=>x.toString(16).padStart(2,'0')).join('');
export async function sha(bytes){return hex(await crypto.subtle.digest('SHA-256',bytes))}
export async function signature(key,canonical){
 const k=await crypto.subtle.importKey('raw',enc.encode(key),{name:'HMAC',hash:'SHA-256'},false,['sign']);
 return hex(await crypto.subtle.sign('HMAC',k,enc.encode(canonical)));
}
export async function verifyGateway(request,env,{now=Date.now()}={}){
 if(typeof env.GATEWAY_HMAC_KEY!=='string'||env.GATEWAY_HMAC_KEY.length<32)throw Error('Unavailable');
 const h=request.headers,time=h.get('x-bedaway-time')||'',nonce=h.get('x-bedaway-nonce')||'',actor=h.get('x-bedaway-actor')||'',origin=h.get('x-bedaway-origin')||'',sig=h.get('x-bedaway-signature')||'';
 if(!/^\d{10}$/.test(time)||Math.abs(Math.floor(now/1000)-Number(time))>60||!/^[a-f0-9-]{36}$/.test(nonce)||!/^[A-Za-z0-9_-]{1,1400}$/.test(actor)||!/^[a-f0-9]{64}$/.test(sig)||origin!==env.GATEWAY_ORIGIN)throw Error('Unauthorized');
 const u=new URL(request.url),bytes=new Uint8Array(await request.arrayBuffer());
 if(bytes.length>1700000)throw Error('Too large');
 const canonical=['bedaway-gateway-v1',time,nonce,request.method.toUpperCase(),u.pathname+u.search,actor,origin,await sha(bytes)].join('\n');
 const key=await crypto.subtle.importKey('raw',enc.encode(env.GATEWAY_HMAC_KEY),{name:'HMAC',hash:'SHA-256'},false,['verify']);
 const rawSig=new Uint8Array(sig.match(/../g).map(x=>parseInt(x,16)));
 if(!await crypto.subtle.verify('HMAC',key,rawSig,enc.encode(canonical)))throw Error('Unauthorized');
 const normalized=actor.replace(/-/g,'+').replace(/_/g,'/'),p=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(Uint8Array.from(atob(normalized),x=>x.charCodeAt(0))));
 if(!p||typeof p.id!=='string'||!/^[A-Za-z0-9_-]{1,100}$/.test(p.id)||typeof p.email!=='string'||p.email.length>320||/[\p{Cc}\p{Cf}]/u.test(p.email))throw Error('Unauthorized');
 return {actor:p,nonce,expires:Number(time)+61,bytes,path:u.pathname+u.search,method:request.method};
}
