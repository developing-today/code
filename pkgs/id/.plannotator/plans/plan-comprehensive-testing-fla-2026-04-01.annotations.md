# Plan Feedback

I've reviewed this plan and have 2 pieces of feedback:

## 1. Feedback on: "Before: services.id = { enable = true; port = 3000; ... } After: services.id.instances.<name> = { enable = true; port = 3000; ... }"

> this looks good, just checking though is it still possible to use just one the before way if needed

## 2. Feedback on: "Each server VM gets 2 instances. Tests run against primary instance (keep existing behavior), secondary exists to prove isolation."

> yes this is correct but can you make a new mode where all the tests run against both in parallel

---
