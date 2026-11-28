# Luna-as-decision harness (Worker C, code#13675 Part 3b)

System under test: OpenAI chat completions, model `gpt-6-luna`, low reasoning
effort (the `openai-payg/gpt-6-luna#low` leg), called directly at
`POST https://api.openai.com/v1/chat/completions`.

## System prompt (verbatim)

You are a decision engine, not a chat assistant. You receive a STATE and one
typed QUESTION. Answer with exactly one JSON object and nothing else, in the
contract shape for the question type:

- noul (yes/no): {"value": true or false, "confidence": <number 0 to 1>}
- choice: {"value": "<exactly one of the listed option ids>", "confidence": <number 0 to 1>}
- score: {"value": <integer level index, 0-based, lowest first>, "confidence": <number 0 to 1>}

Rules: apply any rule or threshold stated in the STATE literally. Pick the
single best-matching option or level from the criteria given. Never invent an
option id or a level outside the given range. Output no prose, no markdown,
no explanation — only the JSON object.

## Wire format

- `response_format`: JSON Schema (strict), schema =
  `{"type":"object","properties":{"value":{},"confidence":{"type":"number","minimum":0,"maximum":1}},"required":["value","confidence"],"additionalProperties":false}`
  with the `value` subschema specialised per item: boolean for noul,
  enum of option ids for choice, integer 0..(levels-1) for score.
- `reasoning_effort`: "low". Temperature omitted (default).
- User message: STATE + QUESTION + criteria rendered verbatim from the eval item.

## Grading (deterministic, scripted — runner.py)

- Schema-faithful: the reply parses as JSON, has exactly the keys
  `value` and `confidence`, `value` has the correct type and lies in the
  allowed set/range, and `confidence` is a number in [0,1].
- Correct: `value` equals the expected value. Score family tolerance:
  correct if within ±1 level of the expected level.
- Baseline (Google Decision 1, native `/v1beta/decisions`): one request per
  item; noul value = (noul >= 0.5), choice value = returned `choice`,
  score value = argmax of returned `probabilities`. A baseline reply counts
  as schema-faithful when the answer object for the question carries the
  documented fields for its type.
