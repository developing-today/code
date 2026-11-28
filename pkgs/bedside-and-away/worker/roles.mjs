/** Store roles (BA-W01 L2). The contract names them viewer, editor and administrator; the database
 * and the existing API store reader, editor and admin. Requests may use either name; responses keep
 * the stored name in `role` and add the contract name as `roleName`. Ownership is not a role. */
export const STORED_ROLES=Object.freeze(['reader','editor','admin']);
const contract=Object.freeze({reader:'viewer',editor:'editor',admin:'administrator'});
const stored=Object.freeze({viewer:'reader',reader:'reader',editor:'editor',administrator:'admin',admin:'admin'});
/** The stored role for a requested role name, or null for anything else. */
export const storedRole=x=>typeof x==='string'&&Object.hasOwn(stored,x)?stored[x]:null;
/** The contract name for a stored role, or null. */
export const roleName=x=>typeof x==='string'&&Object.hasOwn(contract,x)?contract[x]:null;
/** Whether a stored role reaches at least the stored role `level`. */
export const roleAtLeast=(role,level)=>STORED_ROLES.indexOf(role)>=0&&STORED_ROLES.indexOf(role)>=STORED_ROLES.indexOf(level);

/** The automatic name of an account's personal store, from its editable display name (BA-045, BA-080).
 * The name is only a label: the store ID is derived from the account ID and never changes. */
export const personalStoreName=nickname=>nickname?nickname+' personal store':'Personal';

/** Who may seed the Site's default shared store on provisioning (BA-081). `DEFAULT_SHARED_OWNER` unset
 * keeps the prototype rule (the first account to provision seeds it); `none` seeds no shared store;
 * any other value is the one account ID allowed to seed it. Other accounts never join it by provisioning. */
export function maySeedDefaultShared(env,accountId){
 const configured=typeof env?.DEFAULT_SHARED_OWNER==='string'?env.DEFAULT_SHARED_OWNER.trim():'';
 if(!configured)return true;
 if(configured==='none')return false;
 return configured===accountId;
}
