/** Guest claim primitives, NOT an authentication provider or a finished linking route. */
const failure=()=>Object.assign(new Error('Guest claim is invalid or expired'),{status:403});
const text=x=>typeof x==='string'&&x.length>0&&x.length<=200;
const digest=async token=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(token)))).map(b=>b.toString(16).padStart(2,'0')).join('');
export const installationId=()=>crypto.randomUUID();
export async function newGuestClaim({guestId,storeId,at=Date.now(),ttlMs=600000}){
 if(!text(guestId)||!text(storeId)||!Number.isSafeInteger(at)||!Number.isSafeInteger(ttlMs)||ttlMs<1000||ttlMs>600000)throw failure();
 const token=Array.from(crypto.getRandomValues(new Uint8Array(32))).map(b=>b.toString(16).padStart(2,'0')).join('');
 return {token,record:{id:crypto.randomUUID(),guestId,storeId,tokenHash:await digest(token),createdAt:at,expiresAt:at+ttlMs,consumedAt:null}};
}
/** Caller supplies a trusted authenticated account, then atomically consumes record and transfers ownership. */
export async function verifyGuestClaim({record,token,guestId,storeId,accountId,at=Date.now()}){
 if(!record||record.consumedAt!=null||!text(accountId)||record.guestId!==guestId||record.storeId!==storeId||!Number.isSafeInteger(at)||at<record.createdAt||at>=record.expiresAt||typeof token!=='string'||!/^[a-f\d]{64}$/.test(token)||typeof record.tokenHash!=='string'||!/^[a-f\d]{64}$/.test(record.tokenHash))throw failure();
 const hash=await digest(token);let unequal=0;for(let i=0;i<hash.length;i++)unequal|=hash.charCodeAt(i)^record.tokenHash.charCodeAt(i);if(unequal)throw failure();
 return {guestId,storeId,accountId,claimId:record.id};
}
