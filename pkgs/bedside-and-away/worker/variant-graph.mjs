const idOK = x => typeof x === 'string' && x.length > 0 && x.length <= 200 && !/[\p{Cc}\p{Cf}]/u.test(x);
const familyIds = x => [...new Set([...(x.variantFamilyIds || []), x.variantFamilyId].filter(Boolean))];
export function validateVariantGraph(items) {
 if (!Array.isArray(items)) throw Error('Items must be an array');
 const map = new Map();
 for (const item of items) {
  if (!idOK(item.id) || map.has(item.id)) throw Error('Duplicate or invalid item ID');
  if (item.variantFamilyIds !== undefined && (!Array.isArray(item.variantFamilyIds) || item.variantFamilyIds.length > 500 || item.variantFamilyIds.some(x=>!idOK(x)) || new Set(item.variantFamilyIds).size !== item.variantFamilyIds.length)) throw Error('Invalid family associations');
  if (item.variantFamilyId !== undefined && !idOK(item.variantFamilyId)) throw Error('Invalid family ID');
  map.set(item.id,item);
 }
 for (const item of items) {
  const seen = new Set([item.id]); let p=item.parent;
  while(p) {
   if (seen.has(p)) throw Error('Hierarchy cycle');
   seen.add(p); const ancestor=map.get(p);
   if (!ancestor || ancestor.kind !== 'virtual') throw Error('Parent must be an existing general item');
   p=ancestor.parent;
  }
 }
 return true;
}
// Call on generic snapshot mutation; only the dedicated server-planned graph operation may alter associations.
export function assertVariantMetadataUnchanged(before, after) {
 const prior = new Map(before.map(x=>[x.id,x]));
 for (const item of after) {
  const old=prior.get(item.id);
  if (JSON.stringify(familyIds(old||{}).sort()) !== JSON.stringify(familyIds(item).sort()) || (old?.variantFamilyId||null)!==(item.variantFamilyId||null) || (old?.variantSchemaVersion||null)!==(item.variantSchemaVersion||null)) throw Error('Use the variant operation to change family associations');
 }
 return validateVariantGraph(after);
}
export {familyIds};
