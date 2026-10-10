## 2023-10-10 - msgpackr structuredClone performance
**Learning:** Using `structuredClone: true` with msgpackr disables some internal optimizations and is slower, and not needed when not cloning complex maps/sets.
**Action:** Always set `structuredClone: false` for simple MessagePack encoding/decoding unless there's a specific need to clone JS objects with cyclic references or specialized types like Dates.
