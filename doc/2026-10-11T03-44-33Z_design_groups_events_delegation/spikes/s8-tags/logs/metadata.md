## T1. One tag entry, one device: add then clear, repeated

| cycles | ops | live dots | state B | tomb B | log B |
|---:|---:|---:|---:|---:|---:|
| 0 | 1 | 1 | 117 | 118 | 110 |
| 1 | 3 | 1 | 117 | 155 | 365 |
| 10 | 21 | 1 | 117 | 164 | 2660 |
| 100 | 201 | 1 | 119 | 345 | 25964 |
| 1000 | 2001 | 1 | 121 | 3020 | 262554 |
| 10000 | 20001 | 1 | 121 | 30020 | 2629554 |

## T2. Distinct entries, one device: set each once, clear half

| entries | live | state B | state B per live entry | tomb B |
|---:|---:|---:|---:|---:|
| 1 | 1 | 118 | 118.0 | 119 |
| 10 | 5 | 434 | 86.8 | 476 |
| 100 | 50 | 4041 | 80.8 | 4156 |
| 1000 | 500 | 40543 | 81.1 | 41804 |

## T3. Random run, several devices, partial gossip (3000 steps, seed 20261011)

| devices | ops | live dots | vv entries | state B | tomb B | log B | join = reference |
|---:|---:|---:|---:|---:|---:|---:|---|
| 2 | 2947 | 7 | 2 | 611 | 5632 | 456848 | true |
| 5 | 2994 | 26 | 5 | 1835 | 6401 | 612223 | true |

## T4. One clear op: bytes by number of names and devices in the clock

| names | devices in clock | clear op B |
|---:|---:|---:|
| 1 | 1 | 149 |
| 1 | 5 | 301 |
| 1 | 20 | 871 |
| 10 | 1 | 473 |
| 10 | 5 | 625 |
| 10 | 20 | 1195 |
| 100 | 1 | 3791 |
| 100 | 5 | 3943 |
| 100 | 20 | 4513 |

## T5. Compaction of the 5-device run at a causal cut (cut = prefix of issue order)

| cut at | dropped ops | snapshot B | tail B | dropped headers B (32 each) | total B | uncompacted log B |
|---:|---:|---:|---:|---:|---:|---:|
| 50% | 1497 | 3288 | 308768 | 47904 | 359960 | 612223 |
| 90% | 2694 | 4231 | 66886 | 86208 | 157325 | 612223 |
| 99% | 2964 | 1642 | 6323 | 94848 | 102813 | 612223 |

