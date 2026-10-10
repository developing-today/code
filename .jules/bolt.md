## 2023-10-27 - [Hash Calculation Optimization]
**Learning:** `String.prototype.split("").reduce()` is a convenient but slow pattern for string hashing due to array allocations and callback overhead.
**Action:** Use simple `for` loops with `String.prototype.charCodeAt()` when doing character-by-character string hashing or checksums to avoid GC pressure and improve speed by 3-4x.
