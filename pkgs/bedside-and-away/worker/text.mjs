/** Structural text classes (BA-046, BA-W01 L6). Server-side: the browser's own clean-up is a convenience, never the check.
 *
 * - identifier: an opaque ID. Well-formed, 1 to 200 code units, no control or format character (so no
 *   zero-width or bidirectional override character).
 * - name: a label shown to people (store name, nickname, item name). Cleaned rather than refused: control and
 *   format characters and unpaired surrogates are dropped, except inside an emoji sequence (a ZWJ family, a flag
 *   tag sequence) and a zero-width non-joiner between letters, which some scripts need.
 * - comment: free text a person wrote. Refused only when it holds an unpaired surrogate or a control character
 *   other than tab, line feed and carriage return; emoji and every script are accepted as written.
 */
const keep=/(\p{RGI_Emoji})|((?<=[\p{L}\p{M}])‌(?=[\p{L}\p{M}]))|[\p{Cc}\p{Cf}\p{Cs}]/gv;
const wellFormed=s=>!/\p{Cs}/u.test(s);
/** `s` with every invisible character dropped (or replaced) except the joiners that belong to the text. */
export const visible=(s,replacement='')=>s.replace(keep,(m,emoji,zwnj)=>emoji||zwnj?m:replacement);
export const name=(x,max=200)=>visible(String(x??'').normalize('NFC')).trim().slice(0,max).replace(/\p{Cs}$/u,'');
export const isIdentifier=x=>typeof x==='string'&&x.length>0&&x.length<=200&&wellFormed(x)&&!/[\p{Cc}\p{Cf}]/u.test(x);
export const isComment=(x,max=12000)=>typeof x==='string'&&x.length<=max&&wellFormed(x)&&!/[^\P{Cc}\t\n\r]/u.test(x);
