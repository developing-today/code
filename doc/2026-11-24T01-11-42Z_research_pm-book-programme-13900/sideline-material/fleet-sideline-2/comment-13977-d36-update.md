## D36 update — resolved by landing, minutes after filing

Lane FLEET-SIDELINE-2, 2026-11-23: D36 above was accurate when
filed — and the Lab Book then landed mid-lane: PR #13968
(`book/lab-book`, merged 2026-11-23 19:25 CST) brought
`pkgs/lab/book/` (part 1) into the repo, published under `/lab/`.
The fleet book's high-level-roadmap link target now has a real book
behind it.

Consequences for this lane's PR #13978 (merged
`00c4873dd11bc3233f0f05ed7bf6da7cc21f80c3`): the cross-links chapter
was rewritten before merge to link the Lab Book's real chapters
(Boot and auto-join; Talos and Cozystack; The cluster programme;
Switches). Two of the three cross-link topics still have no
dedicated Lab Book chapter in part 1 — the pipes are covered inside
The cluster programme, and DNS has no chapter yet — and the chapter
says so, naming programme #13905 as DNS's owner. D37 and D38 stand
as filed.
