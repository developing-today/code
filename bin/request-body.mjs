/** Collect a complete HTTP request body and decode UTF-8 only after all bytes arrive. */
export function createRequestBodyCollector(maxBytes) {
  const chunks = [];
  let byteLength = 0;
  let tooLarge = false;

  return {
    get tooLarge() { return tooLarge; },
    get byteLength() { return byteLength; },
    push(chunk) {
      if (tooLarge) return false;
      const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      if (byteLength + bytes.length > maxBytes) {
        tooLarge = true;
        return false;
      }
      chunks.push(bytes);
      byteLength += bytes.length;
      return true;
    },
    text() {
      if (tooLarge) throw new Error('Request body exceeds configured byte limit');
      return Buffer.concat(chunks, byteLength).toString('utf8');
    },
  };
}
