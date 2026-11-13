// Public repository fixture only. Never put household notes or private share URLs here.
window.SEED={
 guide:[
  {owner:'Start here',title:'Build a buying guide',body:['Add product options and compare them against the needs of each person. Keep source dates and separate known specifications from unverified fit or performance.'],links:[]},
  {owner:'Product imports',title:'Import a list or a few links',body:['Use Quick submit to preview a pasted list, inspect duplicate links and select the entries to keep. This does not sign in to or retrieve any retailer cart.'],links:[]},
  {owner:'Shared decisions',title:'Compare alternatives',body:['Group variants by the need they satisfy. Ask for changes at the store, category or item level. Requests stay manual until an external connector is configured.'],links:[]}
 ],
 items:[
  {id:'sample-need',name:'Sample travel item',kind:'virtual',tags:['travel'],description:'Example comparison group. Replace or archive it.'},
  {id:'sample-option',name:'Sample option',parent:'sample-need',tags:['travel'],attributes:[['Weight (g)',250]]},
  {id:'sample-alternative',name:'Sample alternative',parent:'sample-need',tags:['travel'],attributes:[['Weight (g)',450]]}
 ]
};
