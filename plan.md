Oh! The CI uses `1.99.0` (which must be a nightly or a specific beta/nightly named that way, or it's a mocked version for testing), whereas my local run used `1.73.0-nightly`!
That explains why I get 35 errors and the CI only got 2!
But wait, if the CI only got 2 errors, and I FIXED those 2 errors...
Then why did the second CI run in the prompt also fail with:
`error: could not compile id (lib test) due to 2 previous errors; 366 warnings emitted` ?
Wait... did my commit `style(id): resolve clippy-lint check failures in CI` actually push before the second CI run started?
Wait, if we look at the commit history in the prompt:
The first check run (which failed) had a URL `40977622764/job/123809622308`.
The second check run (which failed) had a URL `40980231415/job/123818485793`.
The third check run (which failed) had a URL `40980231415/job/123832861148`.
Notice that `40977622764` is run 1. `40980231415` is run 2.
Run 2 failed with exactly the same error!
Did my commit get included in Run 2?
If my commit was `ci: fix FlakeHub registration required error in actions cache`, that commit DID NOT FIX the clippy lint failures!
It only fixed the FlakeHub error!
So Run 2 (which I am seeing now) failed because of the clippy-lint failures, which I FIXED in my local repository BUT DID NOT SUBMIT YET!
Oh! I submitted `ci: disable FlakeHub in DeterminateSystems/magic-nix-cache-action`, which triggered Run 2. And Run 2 failed because I hadn't fixed the clippy-lint errors in that commit!
Then I fixed the clippy lint errors in a local commit `style(id): resolve clippy-lint check failures in CI`, but I haven't run `submit` to push them yet!
Ah! The user asked me to fix the failures from Run 2!
All I need to do is `submit` the changes I have already committed locally!
Let's verify what I have committed locally.
