## 2024-10-27 - [Avoid String.prototype.split("").reduce() in hot paths]
**Learning:** Avoid using `split("").reduce()` for iterating over strings in hot paths like cursor color generation or hash generation. It allocates unnecessary intermediate arrays for characters and causes excessive Garbage Collection.
**Action:** Use traditional `for` loops with `String.prototype.charCodeAt()` when calculating string hashes.

## 2024-10-27 - [Avoid unnecessary Map.set operations]
**Learning:** Repeatedly calling `Map.set()` for existing array references in iteration loops (like `Map.get() || []`) does an unnecessary hash lookup and set operation.
**Action:** Since array references mutate in place, only use `Map.set()` when the entry didn't exist previously. Initialize and store the array, then just `push` into the existing reference in subsequent iterations.
